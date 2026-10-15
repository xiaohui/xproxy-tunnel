# Release signing credentials

Signing is optional. Without these secrets, CI produces unsigned packages for private/internal use; macOS may display a Gatekeeper warning. To enable signatures, add the relevant values under **GitHub → Settings → Secrets and variables → Actions → New repository secret**. Never commit certificate files or paste private keys into issues, pull requests, or chat.

## macOS signing and notarization

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12` certificate, including its private key |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | Exact `Developer ID Application: ...` identity name |
| `APPLE_API_KEY` | App Store Connect API Key ID |
| `APPLE_API_ISSUER` | App Store Connect Issuer ID |
| `APPLE_TEAM_ID` | Apple Developer Team ID |
| `APPLE_API_KEY_CONTENT` | Base64-encoded App Store Connect `.p8` private key |

The workflow uses the API key to notarize and staple each macOS DMG. The certificate must be a Developer ID Application certificate for direct distribution.

On macOS, encode the two files without adding line breaks:

```sh
base64 -i certificate.p12 | tr -d '\n'
base64 -i AuthKey_ABC123.p8 | tr -d '\n'
```

## Windows Authenticode signing

| Secret | Value |
| --- | --- |
| `WINDOWS_CERTIFICATE` | Base64-encoded code-signing `.pfx` containing its private key |
| `WINDOWS_CERTIFICATE_PASSWORD` | Password used when exporting the `.pfx` |

The workflow imports the PFX into the Windows runner certificate store, reads its thumbprint, and passes a temporary Tauri configuration to sign the NSIS installer with SHA-256 and a timestamp server.

Encode the PFX without line breaks, for example on macOS:

```sh
base64 -i codesign.pfx | tr -d '\n'
```

To enable both signing paths, configure all nine secrets. For macOS signing, configure all seven `APPLE_*` secrets together; for Windows signing, configure both `WINDOWS_*` secrets. A partially configured platform fails clearly. With no secrets configured, the workflow creates unsigned packages and keeps the release as a draft for review.
