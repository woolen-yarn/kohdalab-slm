# GUI control API (read only) / GUI 制御 API（読み取り専用）

KohdaLab SLM implements the read-only part of KohdaLab Control Protocol 1, using
the same transport as KohdaLab Camera. It is **off by default**. Without
`--gui-api` the app opens no socket and writes no file in `run/`.

```sh
kohdalab-slm --gui-api --demo            # simulated output, temporary config
kohdalab-slm --gui-api                   # normal GUI, ~/.kohdalab/slm/config/config.json
kohdalab-slm --gui-api --config <path>
```

`--gui-api` cannot be combined with `--demo-capture` or `--demo-smoke`.

## Discovery / 検出

On startup the app writes one owner-only descriptor
`~/.kohdalab/slm/run/slm-<pid>-<instance>.json` (Windows:
`%USERPROFILE%\.kohdalab\slm\run`). It contains `application: "kohdalab-slm"`,
`control_protocol: 1`, `api_version: 1`, the instance UUID, PID, loopback
`host`/`port` and a private `token`. The file is removed when the app closes.
SLM owns this path. The shared crate's application list does not include
`slm`. Choose the descriptor explicitly when several instances run.

From the source checkout, install the same Python SDK used by the other
KohdaLab apps:

```sh
python -m pip install ./bindings/control
```

```python
from kohdalab_control import ControlClient

slm = ControlClient("kohdalab-slm")
print(slm.status())
```

## Wire / 通信

There is one request per TCP connection. The request is a single JSON line
with `control_protocol: 1`, `method: "control.<name>"`, `token` and an
optional `request_id`. The reply is exactly one newline-terminated JSON
record. A wrong token is refused before the request reaches the GUI. A
request that the GUI does not answer within 2 s returns `timeout`. Query again
after closing a modal file dialog.

| Method | Result |
| --- | --- |
| `control.describe` | Application, version, `methods` (only these three), `safety` (`supports_ownership: false`, `read_only: true`, `physical_auto_connect: false`, `device_writes: false`) |
| `control.status` | `phase` (`disconnected`, `initializing`, `ready`, `error`), `busy` (applying, connecting or a wavelength operation in progress), `ownership: {owned: false, supported: false}`, `cleanup` (`not_required`, or `unknown` when the SLM output may be in another state), and `native` (demo/simulated flags, device state, the shown pattern and its receipt, last apply outcome, unknown-output reason) |
| `control.config` | The GUI's in-memory configuration. `saved: false` means it has unsaved edits. Includes profile values, backend, the selected display (name, GDI name, size) and the confirmed input mode |

Any other method, including connect, apply, start, stop, acquire and release,
returns `error.code: "unsupported"`. The adapter does not read any app
state for these requests.

## What is withheld / 公開しない情報

`control.config` omits the config file path, the monitor device path, the
input-mode confirmation note and the free-text calibration note. It reports
only `calibration_note_present`. The withheld fields are listed in
`redacted`. Free-text status fields (labels, errors) replace the home
directory with `~`. The token appears only in the descriptor file.

## Safety boundary / 安全上の境界

- The server thread only queues requests. The GUI answers from the state it
  already holds for that frame. No request sends a worker command, opens or
  moves a display, shows a pattern or writes the wavelength.
- There is no writer lease, so the GUI shows **External control · Read only**
  in the tab row and no Take control or Stop button. Camera shows the same
  indicator.
- `phase: "ready"` and a receipt mean the backend confirmed the draw (demo:
  encoding round-trip; Windows: desktop readback). They do not prove the
  optical response of the SLM.

外部からは状態と設定の読み取りのみ可能です。接続・表示・パターン送信・波長書き込みは
GUI でのみ行います。所有権（リース）がないため、「制御を取得」や「停止」ボタンは
表示しません。

## Tests / テスト

These tests need no hardware: `cargo test -p kohdalab-slm control_api` and
`cargo test -p kohdalab-slm read_only_api`. The second runs the real App on
the demo backend. Its server writes to a temporary run directory, never to
`~/.kohdalab`.
