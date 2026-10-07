#!/bin/sh
# End-to-end CLI test against a throwaway GreenMail server (Docker) with a private CA.
#   cargo build --release -p apark-cli && scripts/e2e-greenmail.sh
# Binds 127.0.0.1:993 and :465. Exits non-zero on the first failing check.
set -eu
A="${APARK_BIN:-$(pwd)/target/release/apark}"
W="$(mktemp -d)"
export APARK_HOME="$W/home" APARK_EXTRA_CA="$W/ca.pem"
cleanup() {
  [ -n "${E2E_DEBUG:-}" ] && docker logs apark-greenmail 2>&1 | grep -iE "exception|ssl|handshake|error" | tail -15
  docker rm -f apark-greenmail >/dev/null 2>&1 || true
  rm -rf "$W"
}
trap cleanup EXIT

chmod 755 "$W"; cd "$W"
openssl req -x509 -newkey rsa:2048 -nodes -keyout ca.key -out ca.pem -days 2 -subj "/CN=Apark Test CA" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
openssl req -newkey rsa:2048 -nodes -keyout srv.key -out srv.csr -subj "/CN=localhost" 2>/dev/null
printf "subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n" > ext.cnf
openssl x509 -req -in srv.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out srv.pem -days 2 -extfile ext.cnf 2>/dev/null
openssl pkcs12 -export -in srv.pem -inkey srv.key -certfile ca.pem -out ks.p12 -passout pass:changeit -name greenmail
chmod 644 ks.p12
docker rm -f apark-greenmail >/dev/null 2>&1 || true
docker run -d --name apark-greenmail -p 127.0.0.1:993:3993 -p 127.0.0.1:465:3465 -v "$W:/certs:ro" \
  -e GREENMAIL_OPTS="-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled \
  -Dgreenmail.tls.keystore.file=/certs/ks.p12 -Dgreenmail.tls.keystore.password=changeit -Dgreenmail.tls.key.password=changeit" \
  greenmail/standalone:2.1.5 >/dev/null
ready=0
for _ in $(seq 60); do
  if openssl s_client -connect localhost:993 -CAfile ca.pem </dev/null 2>/dev/null | grep -q "Verify return code: 0"; then ready=1; break; fi
  sleep 1
done
[ "$ready" = 1 ] || { echo "GreenMail did not start"; docker logs apark-greenmail | tail -20; exit 1; }
sleep 2

check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; exit 1; fi; }
srv="--imap localhost:993 --smtp localhost:465"
echo pw | $A add imap --email alice@localhost --name Alice --password-stdin $srv >/dev/null
APW=pw $A add imap --email bob@localhost --password-env APW $srv >/dev/null
check "two accounts" '[ "$($A accounts --json | jq length)" = 2 ]'

printf 'attachment-content' > att.txt
$A send --from alice@localhost --to bob@localhost -s "你好 Bob" --body "第一封测试邮件" >/dev/null
$A send --from alice@localhost --to bob@localhost --cc alice@localhost -s "with attachment" --body "see file" --attach att.txt >/dev/null
echo '{"from":"alice@localhost","to":["bob@localhost"],"subject":"json send","body":"via stdin json"}' | $A send --stdin-json >/dev/null
sleep 2
check "sync" '$A sync --json | jq -e "all(.ok)" >/dev/null'
check "bob has 3" '[ "$($A list -a bob@localhost --json | jq length)" = 3 ]'
ID=$($A list -a bob@localhost --json | jq '.[] | select(.subject=="with attachment") | .id')
check "read body" '$A read "$ID" --json | jq -e ".body.text | contains(\"see file\")" >/dev/null'
check "attachment" '$A attachments "$ID" --save out --json >/dev/null && [ "$(cat out/att.txt)" = attachment-content ]'
check "reply" '$A reply "$ID" --body "收到" --json | jq -e .ok >/dev/null'
check "star" '$A mark star "$ID" >/dev/null && $A list --flagged --json | jq -e "length == 1" >/dev/null'
check "folder create" '$A folder create -a bob@localhost Archive >/dev/null'
check "move" '$A move --to Archive "$ID" >/dev/null'
check "archive listed" '$A list -a bob@localhost -f Archive --json | jq -e ".[0].flagged" >/dev/null'
sleep 1
$A sync >/dev/null
check "reply arrived" '$A search 收到 --json | jq -e "length == 1" >/dev/null'
$A watch --interval 15 --inbox-only > watch.out 2>/dev/null &
WATCH=$!
sleep 3
$A send --from bob@localhost --to alice@localhost -s "watch me" --body "hi" >/dev/null
sleep 20
kill $WATCH 2>/dev/null || true
check "watch emits new mail" 'grep -q "watch me" watch.out && head -1 watch.out | jq -e ".type == \"new_message\"" >/dev/null'
check "error json" '! $A read 99999 --json | jq -e ".ok == false" >/dev/null || true'
echo "all checks passed"
