# GUI control API / GUI 制御 API

KohdaLab SLM implements KohdaLab Control Protocol 1 with the same transport as
KohdaLab Camera: reads, plus **leased** connect, apply, reset, disconnect and
stop. It is **off by default**. Without `--gui-api` the app opens no socket
and writes no file in `run/`.

```sh
kohdalab-slm --gui-api --demo            # simulated output, temporary config
kohdalab-slm --gui-api --demo-wavelength # same, opened on the wavelength view
kohdalab-slm --gui-api                   # normal GUI, ~/.kohdalab/slm/config/config.json
kohdalab-slm --gui-api --config <path>
```

`--gui-api` cannot be combined with `--demo-capture` or `--demo-smoke`.

Everything the API does goes through the GUI and its single device worker:
the display selected in the GUI, the input mode confirmed in the GUI, the
same display interlock and the same reset-then-close path as the buttons.
The API cannot select or remap a display, confirm an input mode, change the
profile or config, or write the USB wavelength.

## Discovery / 検出

On startup the app writes one owner-only descriptor
`~/.kohdalab/slm/run/slm-<pid>-<instance>.json` (Windows:
`%USERPROFILE%\.kohdalab\slm\run`). It contains `application: "kohdalab-slm"`,
`control_protocol: 1`, `api_version: 1`, the instance UUID, PID, loopback
`host`/`port` and a private `token`. The file is removed when the app closes
or the listener ends. Choose the descriptor explicitly when several instances
run.

Install the common Python SDK (1.0.4, unchanged) from the source checkout:

```sh
python -m pip install ./bindings/control
```

## Python calls / Python からの操作

```python
from kohdalab_control import ControlClient, ControlError

slm = ControlClient("kohdalab-slm")          # or endpoint="<descriptor.json>"
print(slm.describe()["safety"])
print(slm.status())                          # reads need no lease

with slm:                                    # acquire; heartbeat thread starts
    job = slm.invoke("connect")              # GUI-selected display, confirmed mode
    slm.wait(job["job_id"])                  # completed only after the worker ACK

    job = slm.invoke("apply", spec={"kind": "uniform", "code": 512}, label="run 1")
    receipt = slm.wait(job["job_id"])
    print(receipt["native"]["receipt"])      # digest, uniform_code, verification

    job = slm.invoke("apply", item_id="<Patterns row UUID>")
    slm.wait(job["job_id"])

    slm.wait(slm.invoke("reset")["job_id"])  # uniform raw code profile.reset_code
    slm.stop_and_wait()                      # reset, then close; lease kept
# leaving the block releases: release stops (reset, then close) and the SDK
# waits for that stop job's terminal receipt
```

`spec` is a `PatternSpec` in the same JSON form as experiment files, for
example:

```python
{"kind": "single_wave",
 "wave": {"period": {"value": 100.0, "unit": "px"},       # px | um | per_um
          "waveform": "ramp",                             # ramp | binary
          "initial_phase": {"value": 0.0, "unit": "deg"}},  # deg | rad | pi
 "geometry": {"center_offset_px": [-16, 110],
              "rotation": {"value": 15.0, "unit": "deg"}}}  # deg | rad
{"kind": "multi_wave", "components": [<wave>, <wave>], "geometry": {...},
 "stripe_width_px": None, "reverse_band_order": False}
{"kind": "uniform", "code": 0}                            # raw device code
```

It is validated against the GUI's current generation profile before anything
is queued (unknown fields, wrong units, codes above `max_code`, empty or more
than 64 Multi-Wave components are refused with `validation`). `item_id` is
the UUID of an existing row in the GUI's Patterns list (shown on hover); the
row is rendered exactly as **Apply Row** would (imported CSVs are checked by
SHA-256). Give exactly one of `spec` or `item_id`; `label` (1–80 printable
characters) applies only to `spec`.

## Methods / メソッド

| Method | Lease | Answer |
| --- | --- | --- |
| `control.describe` | – | Methods and the safety policy below |
| `control.status` | – | Phase, busy, ownership, cleanup, output (see below) |
| `control.config` | – | The GUI's in-memory config, sanitized |
| `control.result` | – | Receipt of `job_id` (latest 64 jobs) |
| `control.acquire` | – | `lease`, `heartbeat_seconds: 0.5`, `lease_timeout_seconds: 3.0` |
| `control.heartbeat` | yes | Renews the lease (sent by the SDK thread) |
| `control.release` | yes | `released: true`, `job_id` of the stop it started |
| `control.connect` | yes | Job receipt |
| `control.apply` | yes | Job receipt (`spec` or `item_id`, optional `label`) |
| `control.reset` | yes | Job receipt |
| `control.disconnect` | yes | Job receipt |
| `control.stop` | yes | Job receipt; the lease is kept |

Every mutation needs a UUID `request_id` and a `client_id` (the SDK adds
both) and has strict parameters: an unknown parameter is refused. Any other
method returns `unsupported`.

## Semantics / 動作

**Asynchronous outcomes.** A mutation is answered at once with a receipt
`status: "running"`, `terminal: false`, `cleanup: "pending"`. It becomes
terminal only when the device worker acknowledges that exact command
(`ConnectDone`, `Applied`/`ApplyFailed`, `DisconnectDone`). `wait()` raises
on `status: "error"`.

| Job | `completed` | `error` |
| --- | --- | --- |
| connect | Reset frame shown and backend-confirmed; `cleanup: not_required` | Refused or failed. `cleanup: not_required` if nothing was left on screen, `unknown` if the initial reset failed |
| apply, reset | Frame backend-confirmed (`native.receipt`) | `not_required`: not sent, previous frame still shown. `unknown`: draw failed or display lost, output hidden |
| stop, disconnect, release | `verified`: the reset frame was backend-confirmed and the output window closed. `not_required`: nothing was open and nothing was unverified | `unknown`: the reset failed or was impossible (output already unknown), or closing failed |

**Reset code.** The reset frame is the uniform **raw device code**
`profile.reset_code` (provisional `0`, set in Device Settings). It is not a
phase value and is **not verified to be dark**. `verified` and `confirmed`
mean the demo encoding round-trip or the Windows desktop readback. Nothing in
this API measures the optical state (`optical_state: "not_verified"`), and
what the SLM shows after its output window closes is not verified.

**Serial operations.** One operation at a time. While one is waiting for its
acknowledgement (or the GUI is connecting, applying, disconnecting or using
the wavelength), connect, apply, reset and disconnect return `busy`; nothing
is queued. `stop` is the exception: it is accepted while an operation runs,
queued behind it in the worker, and returns the existing stop job if one is
already pending.

**Request IDs.** An accepted mutation's `request_id` is remembered (latest
128) with its method, client and parameters. Sending the same request again
returns the original answer with `duplicate: true` and runs nothing; a
different request with a used ID returns `request_id_conflict`. After a
transport timeout (`outcome_unknown`), resend with the same ID to learn what
was accepted: `slm.invoke("apply", request_id=rid, spec=...)`. Refused
requests and heartbeats are not remembered.

**Display interlock.** `connect` uses only the display selected in the GUI and
the input mode confirmed in the GUI (`connect` takes no parameters). It is
refused with `safety` when the GUI's own Connect would be blocked (no
selection, not found, primary, wrong size, unconfirmed or 120 Hz mode, remote
session, …). The worker then re-enumerates displays before connect and every
draw and hides the output on any doubt, exactly as for the buttons.

## Lease, release, expiry / 所有権・解放・失効

- `acquire` is refused with `ownership` while another client holds the lease,
  with `busy` while a previous operation or lease-end stop is still waiting
  for the worker or the GUI is operating the device, and with
  `cleanup_unconfirmed` while the output is unknown or unverified. Only a
  backend-confirmed **Connect in the GUI** clears an unverified output.
- **Release** always stops: the lease ends at once and a stop job (reset,
  then close) starts. The SDK follows that job and raises
  `cleanup_unconfirmed` if its cleanup is not `verified`/`not_required`.
- **Expiry**: when the GUI processes a missed heartbeat deadline (3 s),
  it revokes the lease and requests the reset and close
  (`origin: "lease_expired"`).
- **Transport loss**: there is no persistent connection, so a lost client is
  seen as missed heartbeats (expiry). If the listener itself ends, the lease
  is revoked and the output stopped (`origin: "listener_ended"`).
- **Revoked leases**: requests carrying an ended lease get
  `error.code: "ownership"`, `application_code: "revoked"` with the reason
  (`released`, `expired`, `gui_take_control`, `gui_stop`,
  `listener_ended`). The SDK keeps a failed heartbeat and raises it on the
  next `invoke`/`acquire`; create a new `ControlClient` to take control again.
- `status.ownership` reports `owned`, `supported: true`, the timing and
  `last_revocation`. The lease token never appears in status or logs.

File dialogs are disabled while a script holds the lease or an external
operation is pending, so they cannot delay Stop. Heartbeat expiry and
request dispatch require a responsive GUI.

## GUI / 画面

The tab row shows **External control · Available**, **· Python** (leased),
**· Waiting for device** or **· Output unverified**, with two buttons:

- **Stop** (always available while anything is open or leased): ends any
  lease, sends the reset pattern, then closes the output.
- **Take control** (while leased): ends the lease and leaves the output as it
  is. An operation already sent still finishes and gets its receipt.

While a client holds the lease, or its operation or stop is unacknowledged,
the manual Connect, Disconnect, Reset Pattern, Apply, Apply Row,
Previous/Next, auto-apply clicks, display, backend and mode selection, config
Load, Device Settings and wavelength controls are locked. Pattern editing and
the Patterns list stay available.

## Status / 状態

`native.output` and `cleanup` are conservative:

| `native.output` | `cleanup` | Meaning |
| --- | --- | --- |
| `confirmed` | `not_required` | Connected and the backend confirmed the frame in `native.showing` |
| `none` | `not_required` | Disconnected, with no unresolved unknown output. Not verified optically |
| `pending` | `pending` | A connect, apply/reset, disconnect or stop is in flight. `native.showing` is `null`; the last confirmed frame is in `native.previous_showing` and may no longer be shown |
| `unknown` | `unknown` | A draw or reset failed or the display was lost. `phase` is `error`, also after disconnecting, until a backend-confirmed connect |

`busy` is true while any operation is in flight. `native.operation` names the
job still waiting for the worker. `native.reset_code` and
`native.reset_meaning` state the reset frame.

`control.config` omits the config file path, the monitor device path, the
input-mode confirmation note and the free-text calibration note (listed in
`redacted`). Free-text fields replace the home directory with `~`.

外部クライアントはリース取得後に接続・パターン送信・リセット・停止を行えます。表示先と
入力モードはGUIで選んだものだけを使い、APIから変更できません。解放・失効・通信断では
リセットパターンを送ってから出力を閉じます。リセットはプロファイルのリセットコード
（生のデバイスコード）であり、光学的に暗いことは確認していません。GUIの「停止」と
「制御を取得」は常に操作者が使えます。

## Tests / テスト

No hardware is used: `cargo test -p kohdalab-slm control` (adapter, lease
engine, spec validation), `cargo test -p kohdalab-slm leased`,
`… serial`, `… expiry`, `… take_control`, `… unknown_output`,
`… failed_reset`, `… ended_listener` and `… python_sdk` run the real App on
the demo backend with servers in a temporary run directory, never
`~/.kohdalab`. `python_sdk_drives_the_demo_gui_without_real_output` runs
`bindings/control/slm_gui_smoke.py` with the unmodified SDK over the real
socket (`PYTHON` selects the interpreter); the script refuses to run unless
status reports simulated output. `cargo test -p slm-device worker` covers the
worker acknowledgements. The SDK's own tests run with
`python -m unittest -v test_control` from `bindings/control`.
