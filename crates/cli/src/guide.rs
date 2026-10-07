pub const GUIDE: &str = r#"# Apark CLI — guide for AI agents

All commands accept `--json` (or env APARK_JSON=1) for machine-readable output.
On failure: exit code 1 and, in JSON mode, `{"ok": false, "error": "..."}` on stdout.
Reads come from the local cache (instant); run `apark sync` (or keep `apark daemon`
running) to refresh. Message IDs are local integers from `list`/`search`.

## Accounts
  apark login [--manual]                 Sign in with the master Google account; restores all
                                         accounts synced to its Google Drive app folder.
  apark add google|microsoft [--manual]  OAuth account. --manual = no browser: open the URL
                                         anywhere, paste the 127.0.0.1 redirect URL back.
  apark add imap --email E --password-env VAR [--imap host:993] [--smtp host:465]
  apark add imap --email E --password-stdin   (password on first stdin line)
  apark accounts | apark remove E
  apark cloud status | sync | push | off  Account-list sync (any device restores all accounts).
  apark cloud server --url URL --user U [--token T] --password-env VAR
                                         Sync via a self-hosted `apark server` (E2E encrypted).
  apark cloud webdav --url FILE_URL --user U --password-env VAR   (needs APARK_SYNC_PASSPHRASE)
  apark cloud file --path PATH           File in iCloud Drive / Dropbox / Syncthing (needs passphrase)
  apark server [--listen 0.0.0.0:8787] [--data DIR] [--token T]
                                         Run the self-hosted sync server.

## Reading
  apark sync [-a E] [-f FOLDER]...       Fetch new mail (INBOX + extra folders).
  apark list [-a E] [-f FOLDER] [-c people|notification|newsletter] [--unread] [--flagged]
             [--limit N] [--sync]
  apark search WORDS... [-a E] [-f FOLDER] [--limit N]   (all words must match)
  apark read ID [--html] [--raw] [--mark-read]
  apark folders [-a E]
  apark folder create|delete -a E NAME | apark folder rename -a E OLD NEW
  apark attachments ID [--save DIR] [--index N]

  Message JSON: {id, account, folder, uid, message_id, subject, from_name, from_addr,
                 to, cc, date (unix), seen, flagged, category, snippet, has_body}
  read --json:  {"message": {...}, "body": {"text", "html", "attachments": [{name,size}]}}

## Acting
  apark mark read|unread|star|unstar ID...
  apark archive ID... | apark trash ID... | apark move --to FOLDER ID...
  apark send --from E --to A [--to B] [--cc C] [--bcc D] -s SUBJECT
             (--body TEXT | --body-file F | stdin) [--html-file F] [--attach F]...
  echo '{"from":"E","to":["A"],"subject":"S","body":"..."}' | apark send --stdin-json
  apark reply ID [--all] [--no-quote] (--body TEXT | stdin) [--attach F]...
  apark forward ID --to A [--body TEXT]

## Headless
  apark daemon [--interval SECS]         Keep syncing in the background (JSON lines with --json).
  apark watch [--interval SECS] [--inbox-only]
                                         Keep syncing; print one JSON line per new message:
                                         {"type":"new_message","message":{...}}
  apark config show | set KEY VALUE | unset KEY
  Data dir: APARK_HOME (default: platform data dir/Apark). OAuth client: APARK_GOOGLE_CLIENT_ID,
  APARK_GOOGLE_CLIENT_SECRET, APARK_MS_CLIENT_ID. Cloud encryption: APARK_SYNC_PASSPHRASE.
"#;
