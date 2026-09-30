<!-- oma:changes:start -->
<!-- Write the changes here -->
<!-- oma:changes:end -->

<!-- oma:generated:start -->
## Install

Download `OpenMonitor.Advanced_{{VERSION}}_x64-setup.exe` and run it. For a silent install use `/S`; add `/NOSENSORS` to leave out the Advanced sensors component:

```
OpenMonitor.Advanced_{{VERSION}}_x64-setup.exe /S /NOSENSORS
```

{{SIGNING}}

See [CODE_SIGNING.md](https://github.com/Cioscos/OpenMonitorAdvanced/blob/main/CODE_SIGNING.md) for how releases are signed.

## Verify your download

Compare the SHA-256 of the installer with the one in `SHA256SUMS.txt`:

```powershell
Get-FileHash .\OpenMonitor.Advanced_{{VERSION}}_x64-setup.exe -Algorithm SHA256
```

Verify the build provenance attestation:

```
gh attestation verify OpenMonitor.Advanced_{{VERSION}}_x64-setup.exe --repo Cioscos/OpenMonitorAdvanced
```
<!-- oma:generated:end -->
