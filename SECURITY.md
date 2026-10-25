# Security

Splitwire runs a privileged Windows service, so security reports matter a lot.

## Reporting a vulnerability

Please **don't open a public issue**. Use GitHub's private reporting instead: **Security → Report a vulnerability** on this repository. Include steps to reproduce and the version you tested. You'll get a reply as soon as possible.

## How Splitwire is meant to be safe

- **Small privileged surface.** Only `splitwire-service` runs as SYSTEM. The app runs as the signed-in user.
- **Restricted pipe.** The service's named pipe accepts SYSTEM, Administrators and interactive users only. Network logons are refused.
- **No raw engine config from clients.** Clients send a typed, validated tunnel description, and the service generates the sing-box config itself, so a client can't make sing-box read or write arbitrary files.
- **Keys stay off disk.** Private keys reach sing-box over stdin, never in a file. Saved tunnels are encrypted with Windows DPAPI for the user's account.
- **Engine API locked down.** sing-box's local stats API listens on a random localhost port with a random per-session secret.
- **No orphaned engine.** sing-box runs inside a kill-on-close job object, so it can't outlive the service.
- **Signed updates.** In-app updates are checked against the public key built into the app before the installer runs. Signatures are bound to their version, so a manifest can't pair a new version number with an old (or different) installer.

Anything that breaks one of these is a vulnerability worth reporting.
