# Product Design visual QA

final result: blocked

Source visual truth:
- `design/home-reference.png` — selected second homepage result, 1487 × 1058 pixels.
- `design/settings-reference.png` — user-attached guided configuration reference, 1487 × 1058 pixels.

Target viewport: match source pixels at density 1 for comparison; native initial window is 1200 × 860 with responsive layout.

Implementation screenshot: unavailable.

State: homepage connected reference uses example values; desktop must be populated only by verified runtime status. An explicitly labelled browser-only `?preview=connected` fixture exists for visual comparison. The settings page defaults to unknown/disconnected data.

Full-view comparison: blocked because the browser could not open `http://127.0.0.1:4175`. The user granted local preview permission, but the retry remained blocked by a saved permission or managed policy. No alternate surface or protocol was used to bypass it.

Focused region comparison: blocked for the same reason. Required later regions are status/switch/metrics, proxy scope, SSH inputs and setup sidebar.

Required fidelity surfaces:
- Fonts / typography: implemented system sans stack; rendered match has not been inspected.
- Spacing / layout rhythm: reference-based homepage sidebar and full-width guided settings implemented; rendered match has not been inspected.
- Colors / tokens: white/gray surfaces, blue accent and semantic status colors implemented; rendered contrast has not been inspected.
- Image / asset fidelity: supplied application icon reused; standard Phosphor UI icons. No custom-drawn illustrations or placeholder assets. Rendered fidelity unverified.
- Copy / content: clarified system-proxy scope, separate Chrome/Edge window, unknown metrics and actual state; rendered wrapping unverified.

Intentional behavior corrections to generated mock:
- One proxy main switch. Automatic reconnect is always enabled while desired state is on.
- Config fields are saved before selecting scope; final setup action connects after both steps.
- System proxy affects applications honoring it, not every application.
- The browser is a separate installed Chrome/Edge, not embedded into Tauri.
- No fabricated live status or latency in the native application.

Comparison history: no visual comparison was possible. No visual acceptance or fidelity pass is claimed.

Implementation checklist:
1. Resolve the saved browser permission or managed policy blocking the local preview. User permission has already been granted.
2. Inspect home and configuration at the source viewport and the native initial size.
3. Verify navigation, controls, file-picker boundary, errors and retry states.
4. Capture implementation screenshots, combine with source references in the same input, fix P0/P1/P2 discrepancies and repeat.
