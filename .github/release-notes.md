## Install

1. Download **`Splitwire_<version>_x64-setup.exe`** below.
2. Run it. Windows may show "Windows protected your PC" because the installer isn't code-signed yet. Click **More info → Run anyway**.
3. Approve the administrator prompt. Splitwire installs a small background service that runs the tunnel, so the app itself never needs admin rights.

Coming from TunnlTo or WireSock? Turn them off first, then import your `.conf`. Your `AllowedApps` / `DisallowedApps` lists carry over.

Verify your download with `SHA256SUMS.txt`:

```powershell
Get-FileHash .\Splitwire_*_x64-setup.exe -Algorithm SHA256
```
