# Clipboard Relay

Clipboard Relay is an explicit, one-item text transfer path between TieZ desktop devices. It is deliberately separate from cloud history synchronization.

## Behavior

- Configure the same WebDAV account and relay shared key on each device.
- Assign the **Send clipboard relay** and **Fetch clipboard relay** global shortcuts. Both shortcuts are empty by default.
- Send uploads only the current `text/plain` clipboard value. It does not upload clipboard history, tags, pin state, usage metadata, settings, or local file paths.
- Fetch copies only the newest eligible item to the operating-system clipboard. TieZ marks this app-originated write so it is not added to local history.
- Messages expire after ten minutes. Each receiving device writes its own acknowledgement, and a local receipt ledger prevents an acknowledged or partially acknowledged item from overwriting the clipboard twice.

## Security model

- Relay traffic uses an independent `<base-path>/relay/v1` WebDAV namespace.
- A 32-byte shared key encrypts message text with XChaCha20-Poly1305. Authenticated data binds the message ID, sender, target list, media type, timestamps, algorithm, and WebDAV file name.
- Acknowledgements are encrypted and authenticated with the same relay key. Unauthenticated ACK files are rejected.
- Relay envelopes contain no plaintext content hash or other plaintext fingerprint of the clipboard text.
- The shared key is kept in the operating-system credential store (Windows Credential Manager, macOS Keychain, or Linux Secret Service). It is not returned by the general settings API and is never stored as a normal setting.
- Generating a key shows it once after the native credential-store write succeeds. Save it securely and import it on the other devices.
- Portable mode is disabled for Relay rather than falling back to plaintext key storage.
- Relay requires an `https://` WebDAV URL. Application-layer encryption does not protect WebDAV credentials, object paths, or traffic patterns from an insecure transport.

## Limits and delivery semantics

- UTF-8 text only; 64 KiB maximum. Leading/trailing whitespace and line endings are preserved.
- The remote queue is capped at 2,000 message objects. Normal send/fetch maintenance removes expired and excess oldest objects on a best-effort basis.
- Default retention is ten minutes; protocol validation rejects TTLs over 24 hours and excessive clock skew.
- Delivery is at-most-once from TieZ's local perspective. If clipboard write succeeds but ACK upload fails, the receipt stays `copied_pending_ack`; later fetches retry the ACK without copying the text again.
- Corrupt, tampered, wrong-key, and unauthenticated objects are reported as errors rather than silently appearing as an empty inbox.

## Linux note

Linux needs a running user D-Bus Secret Service implementation, such as GNOME Keyring or KWallet. If it is unavailable, Relay fails closed while the rest of TieZ continues to work.
