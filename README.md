# codename-fleet

> **สถานะ: alpha (0.2.1)** — คำสั่งและ flag ของ Orca ตรวจกับ **Orca 1.4.224** แล้ว (schema ใน `docs/orca-1.4.224/` + ทดสอบสดบน Windows) · รูปแบบ JSON ของบางคำสั่งยังอ่านแบบยืดหยุ่น (ดูหัวข้อ "สถานะการตรวจ")

`codename` (ชื่อย่อ `cn`) คือเครื่องมือบรรทัดคำสั่งไฟล์เดียว (`codename.exe`) ที่ต่อยอดแอป **Orca** ([stablyai/orca](https://github.com/stablyai/orca)) ให้กลายเป็น **กองเรือ codename ที่มีชื่อ** — agent AI (Claude Code, Codex, Grok, …) แต่ละตัวมีชื่อ ตัวตน บ้าน ความจำ และคุยกันเองได้ ทุกตัวที่เกิดผ่านเครื่องมือนี้เป็นสมาชิก **เผ่า codename**

Orca มี terminal, worktree, orchestration และการรอให้ agent ทำงานเสร็จอยู่แล้ว `codename` เติมชั้น "กองเรือ" ที่ Orca ไม่มี:

- **ทะเบียนกองเรือ** (`fleet.json`) — ใครชื่ออะไร engine อะไร บ้านที่ไหน ยังรันอยู่ไหม
- **เปิดที่นั่งด้วยชื่อ** และตอบหน้าต่างถามครั้งแรกที่ปลอดภัยให้ (trust folder) — หยุดให้มนุษย์ตัดสินเองเมื่อเป็นเรื่องล็อกอินหรือ hook
- **พิธีกำเนิด** codename ใหม่ (SOUL.md, CLAUDE.md, AGENTS.md, BIRTH.md, โฟลเดอร์ ψ, ทารกเลือกชื่อเองได้)
- **ความจำกลาง** ผ่าน arra (ไม่บังคับ)
- **ฟื้นทั้งกองเรือหลังรีบูต** ด้วยคำสั่งเดียว
- **ตรวจสุขภาพ** รวมถึงจับว่า Orca แอบแก้ไฟล์ config ของ agent เพิ่มหรือไม่

## สิ่งที่ต้องมี

- Windows 10 / 11 (คอมไพล์และเทสต์บน macOS/Linux ได้ด้วย)
- **Orca** 1.4.224 ติดตั้งแล้ว และ CLI `orca` ใช้ได้ — ต้องมี **runtime ของ Orca** ทำงานอยู่ อย่างใดอย่างหนึ่ง:
  - แอป Orca (GUI) เปิดอยู่ หรือ
  - runtime แบบไม่มีหน้าจอ `orca serve --no-pairing` ผ่าน scheduled task ที่ `codename serve-task install` สร้างให้ (**ไม่ต้องเปิด GUI**)
- **Git**
- agent CLI อย่างน้อยหนึ่งตัว เช่น Claude Code (`claude`) หรือ Codex (`codex`) — ล็อกอินเองให้เรียบร้อย
- (ไม่บังคับ) **arra** memory server ที่ `http://127.0.0.1:47778`

## ติดตั้ง

PowerShell (ไม่ต้องใช้สิทธิ์ admin):

```powershell
irm https://raw.githubusercontent.com/oldevill14/codename-fleet/main/install.ps1 | iex
```

สคริปต์จะดาวน์โหลด release ล่าสุด ตรวจ SHA256 ติดตั้ง `codename.exe` และ `cn.exe` ลง `%USERPROFILE%\.local\bin` และเพิ่มโฟลเดอร์นั้นเข้า PATH ของผู้ใช้ (ไฟล์เก่าถูกเปลี่ยนชื่อเป็น `.bak` ไม่ถูกลบ)

แล้วเปิด terminal ใหม่:

```powershell
setx ORCA_TELEMETRY_DISABLED 1
codename setup
codename doctor
```

### คอมไพล์เอง

```powershell
cargo build --release
# ได้ target\release\codename.exe — ก๊อปไปไว้ใน PATH และก๊อปอีกชุดชื่อ cn.exe
```

ต้องมี Rust stable (และ Visual Studio Build Tools บน Windows)

## คำสั่ง

ทุกคำสั่งรับ `--json`, `--verbose` (พิมพ์คำสั่ง orca ที่รันและบรรทัด log ออก stderr) และ `--help`

**คำสั่งที่ห่อของที่ Orca มีอยู่แล้ว — ใช้ชื่อเดิม ไม่มีชื่ออื่น**

| คำสั่ง | ทำอะไร |
|---|---|
| `codename hey <ชื่อ> "<ข้อความ>"` | ส่งข้อความแล้วไม่รอ (สำเร็จเมื่อเห็นว่า agent เริ่มทำงาน) |
| `codename wait <ชื่อ> [--timeout 600]` | รอจนเสร็จ · exit 0 เสร็จ · 2 หมดเวลา · 3 ติดค้างรอคนตอบ |
| `codename ask <ชื่อ> "<ข้อความ>"` | ส่ง รอ แล้วพิมพ์เฉพาะคำตอบรอบล่าสุด (exit code เหมือน wait) |
| `codename peek <ชื่อ> [-n 40]` | ดูหน้าจอล่าสุด |
| `codename wt <ชื่อ> <slug>` | สร้าง worktree แยกให้ codename ทำงานโค้ด |
| `codename team up <charter.yaml>` / `team status` / `team down` | ทีมจาก charter (ดู `examples/charter.yaml`) |

**คำสั่งใหม่ (Orca ไม่มี) — ชื่อเต็ม + alias ชื่อย่อเดิม**

| คำสั่ง | alias | ทำอะไร |
|---|---|---|
| `codename setup` | `init` | ตั้งค่าครั้งแรก: โฟลเดอร์ `.codename`, สำรอง config ของ agent + manifest, terminal ตัวตนกัปตัน |
| `codename setup --free` | `init --free` | เพิ่ม engine โมเดลฟรี `claude-free` / `codex-free` ผ่าน Free Claude Code (เครื่องมือแยก ติดตั้งเองค่ะ ดู [โหมดฟรี](docs/USAGE-th.md#โหมดฟรี-free-claude-code)) |
| `codename startup` | `up` | ฟื้นทั้งกองเรือด้วยคำสั่งเดียว: ถ้า runtime ของ Orca ไม่ตอบ จะรัน scheduled task (`serve_task`) หรือ `orca open` แล้วรอสูงสุด 60 วิ |
| `codename shutdown` | `down` | ให้ทุกตัวพัก (ลูกทีมก่อน กัปตันหลังสุด) ไม่ปิดแอป Orca |
| `codename list` | `ls` | ตารางกองเรือ + สถานะจริง (rest / idle / working / blocked) |
| `codename birth <ชื่อ\|--choose> --engine <e> --role "<หน้าที่>"` | `bud` | ให้กำเนิด codename ใหม่ |
| `codename wakeup <ชื่อ> [--fresh]` | `wake` | เปิดที่นั่ง (ไม่เปิดซ้ำถ้ามีอยู่แล้ว) · ถ้าเคยตื่นแล้วจะ **คุยต่อจากรอบก่อน** (`claude --continue`, `codex resume --last`) เว้นแต่ใส่ `--fresh` |
| `codename sleep <ชื่อ>` | `rest` | ปิดที่นั่ง บ้านและความจำอยู่ครบ |
| `codename report "<ข้อความ>"` | `done` | รายงานกลับหากัปตัน `REPORT <ชื่อ> rc=<code>: ...` |
| `codename remember "<บทเรียน>"` | `learn` | จดลงความจำกลาง (arra) |
| `codename recall "<คำค้น>"` | `search` | ค้นความจำกลาง |
| `codename doctor` | `check` | ตรวจสุขภาพทั้งระบบ (รวมสถานะ runtime ของ Orca ผ่าน `orca status`: แอปหรือ serve) |
| `codename serve-task install\|remove\|status` | `svc` | สร้าง/ลบ/ดู scheduled task ของ Windows ที่รัน `orca serve --no-pairing --json` ตอน logon ในเซสชันของผู้ใช้ |

คู่มือภาษาไทยพร้อมตัวอย่าง: [`docs/USAGE-th.md`](docs/USAGE-th.md) · การออกแบบ: [`docs/DESIGN.md`](docs/DESIGN.md)

## ข้อควรระวัง

- **Orca ใส่ hook ลงในไฟล์ config ของ agent** (`~/.claude/settings.json`, `~/.codex/hooks.json` ฯลฯ) — `codename setup` สำรองไฟล์เหล่านี้พร้อม SHA256 ไว้ใน `.codename\backups\` และ `codename doctor` จะแจ้งเมื่อไฟล์เปลี่ยน · ตัดสินใจเองว่าจะเก็บหรือถอด hook
- **ปิด telemetry**: ตั้ง `ORCA_TELEMETRY_DISABLED=1` และปิดใน Orca Settings → Privacy
- **ต้องมี runtime ของ Orca** — แอป Orca หรือ `orca serve` (`codename serve-task install`) · `orca serve` ที่สั่งผ่าน SSH จะตายเมื่อ SSH หลุด จึงให้รันเป็น scheduled task ในเซสชันของผู้ใช้
- **`orca serve` เปิด websocket ที่ 0.0.0.0:6768** — **ห้ามเปิดพอร์ตนี้ใน Windows Firewall** และต้องใช้ `--no-pairing` เสมอ (codename ใส่ให้ตายตัว)
- **กับดัก `--from`**: คำสั่ง orchestration ของ Orca ต้องมี terminal ผู้ส่ง ไม่งั้นได้ `no_active_sender_terminal` แม้ help จะบอกว่าไม่บังคับ — `codename` ใส่ `--from <terminal กัปตัน>` ให้ทุกครั้ง (terminal นี้สร้างตอน `setup`)
- **ห้ามให้ Orca เปิด agent ตัวเดียวกันซ้ำ** กับที่นั่งที่ `codename` คุมอยู่ — จะได้สองที่นั่งชื่อเดียวกัน (`doctor` จะเตือน)
- **ไม่รัน `orca` ที่เป็น `.cmd`/`.bat`**: codename หา `orca` ใน PATH เอง (ตามลำดับ PATHEXT) ถ้าเจอเป็นสคริปต์ batch จะปฏิเสธ เพราะ Windows รันผ่าน `cmd.exe` ซึ่งทำให้ข้อความที่ส่งแทรกคำสั่งได้ (CVE-2024-24576 "BatBadBut") — ให้ตั้ง `CODENAME_ORCA_BIN` หรือ `orca_bin` ใน `config.json` ชี้ไปที่ `.exe` ตัวจริง · ไม่เคยรันผ่าน `cmd /C` · `codename doctor` บอก path และชนิดของ orca ที่เจอ
- **อย่าวาง ROOT หรือบ้านของ codename ใน OneDrive** (หรือโฟลเดอร์ซิงก์คลาวด์อื่น) — การซิงก์ล็อก/เขียนทับไฟล์ `.git` ได้ · `setup`, `birth`, `doctor` จะเตือน และแจ้ง (แค่ข้อมูล) ถ้า path มีตัวอักษรไม่ใช่ ASCII หรือมีช่องว่าง
- **log**: ทุกคำสั่งถูกบันทึกเป็น JSON-lines ที่ `.codename\logs\codename-YYYY-MM-DD.log` (เวลา คำสั่ง ชื่อสมาชิก codename argv ของ orca exit code ระยะเวลา — ข้อความยาวเกิน 200 ตัวอักษรถูกตัด) เก็บ 14 วัน ไฟล์เก่าย้ายไป `logs\old\` ไม่ลบ
- `team down` ไม่ลบ worktree ที่มีงานยังไม่ commit — จะ commit WIP ลง branch ของมันก่อน ถ้าหา path ไม่เจอจะเก็บ worktree ไว้

## สถานะการตรวจกับ Orca

- **ตรวจแล้วกับ Orca 1.4.224**: ทุกคำสั่งย่อยและ flag ที่ codename ส่ง (เทสต์ตรวจกับ `docs/orca-1.4.224/agent-context.json` ทุกครั้งที่ `cargo test`) และ JSON ของ `terminal create/send/wait/read/close` (ทดสอบสดบน Windows — `docs/orca-1.4.224/live-roundtrip.txt`)
- **ยังอ่านแบบยืดหยุ่น (ยังไม่เห็นของจริง)**: JSON ของ `status`, `repo add`, `terminal list`, `worktree create/list/rm` และคำสั่ง orchestration · คำสั่ง resume ของ Codex (`codex resume --last`) แก้ได้ใน `engines.json`
- ถ้า Orca รุ่นใหม่เปลี่ยน flag: รัน `orca <คำสั่ง> --help` จดลง `docs/orca-cli-observed.md` แล้วแก้ตารางหัวไฟล์ `src/orca.rs` ที่เดียว · ตั้ง `CODENAME_ORCA_BIN` (หรือ `orca_bin` ใน config.json) ได้ถ้า binary ไม่ได้ชื่อ `orca`

---

## English

**Status: alpha.** `codename` (short: `cn`) is a single-binary CLI that turns the **Orca** desktop app ([stablyai/orca](https://github.com/stablyai/orca)) into a **fleet of named codenames**: every agent (Claude Code, Codex, Grok, …) gets a name, an identity, a home repo and shared memory, and agents can message each other. Every member of the fleet is called a **codename**.

**Requirements:** Windows 10/11 (also builds on macOS/Linux), Orca 1.4.224 with its `orca` CLI and a running Orca runtime — either the desktop app or the headless `orca serve --no-pairing` started by a scheduled task (`codename serve-task install`; the GUI is optional), Git, at least one agent CLI you are logged in to, optionally an arra memory server on `127.0.0.1:47778`.

**Install:** `irm https://raw.githubusercontent.com/oldevill14/codename-fleet/main/install.ps1 | iex` (no admin; verifies SHA256; installs `codename.exe` + `cn.exe` to `%USERPROFILE%\.local\bin`). Build from source: `cargo build --release`.

**Commands:** wrappers around Orca features keep their original names with no alias — `hey`, `wait`, `ask`, `peek`, `wt`, `team up|status|down`. New commands use full names with the old short name as a visible alias — `setup`/`init`, `startup`/`up`, `shutdown`/`down`, `list`/`ls`, `birth`/`bud`, `wakeup`/`wake`, `sleep`/`rest`, `report`/`done`, `remember`/`learn`, `recall`/`search`, `doctor`/`check`, `serve-task`/`svc` (install|remove|status of the Windows logon task running `orca serve --no-pairing --json`). `startup` starts the runtime when `orca status` says it is down (scheduled task if configured, else `orca open`, then waits up to 60 s); `doctor` reports the runtime state and whether the app or `serve` hosts it. `wait` and `ask` exit 0 (done), 2 (timeout), 3 (blocked). `wakeup` resumes the previous conversation once a codename has been woken before (`claude --continue`; `codex resume --last` is unverified) unless `--fresh`; `startup` always resumes. Every command takes `--json` and `--verbose`.

**Safety:** Orca injects hooks into agent config files, so `setup` backs them up with a SHA256 manifest and `doctor` reports drift. Set `ORCA_TELEMETRY_DISABLED=1`. An Orca runtime must be running (app or `orca serve`). `orca serve` listens on 0.0.0.0:6768: never open that port in the Windows firewall; `--no-pairing` is always passed. Orchestration needs `--from` (handled for you). Never let Orca start an agent that `codename` already runs. codename resolves `orca` on PATH itself (PATHEXT order) and refuses `.cmd`/`.bat` shims (CVE-2024-24576 "BatBadBut" argument injection via `cmd.exe`); point `CODENAME_ORCA_BIN` or `orca_bin` in config.json at the real `.exe`. It never spawns through `cmd /C`. `setup`, `birth` and `doctor` warn when the fleet root or a home is inside OneDrive, and note non-ASCII characters or spaces. Every command is logged as JSON lines to `.codename/logs/codename-YYYY-MM-DD.log` (strings truncated to 200 chars; files older than 14 days are moved to `logs/old/`, never deleted).

**Verified against Orca 1.4.224.** Every Orca subcommand and flag codename sends is checked by a test against the bundled schema (`docs/orca-1.4.224/agent-context.json`), and terminal create/send/wait/read/close were round-tripped live on Windows. JSON results of status, repo add, terminal list, worktree and orchestration commands are still parsed leniently. All flags live in one table at the top of `src/orca.rs`.

License: MIT.
