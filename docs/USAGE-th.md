# คู่มือใช้งาน codename (ฉบับย่อ)

> ใช้ชื่อเต็มของคำสั่งในเอกสารและใน CLAUDE.md/AGENTS.md ของ codename · ชื่อย่อ (alias) พิมพ์ได้เหมือนกัน · ใช้ `cn` แทน `codename` ได้

## เริ่มครั้งแรก

```powershell
setx ORCA_TELEMETRY_DISABLED 1      # แล้วเปิด terminal ใหม่
codename setup                      # หรือ: codename setup --root D:\fleet
codename doctor
```

`setup` จะ: สร้าง `%USERPROFILE%\.codename\`, สำรองไฟล์ config ของ agent พร้อม SHA256 (ครั้งแรกครั้งเดียว ใช้ `--rebaseline` ถ้าต้องการถ่ายใหม่), สร้าง `engines.json`, และเปิด terminal ตัวตนของกัปตัน `codename-captain` (shell เปล่า ใช้เป็น `--from`)

## ให้กำเนิด codename

```powershell
codename birth --choose --engine claude --role "กัปตันของเครื่องนี้" --captain
codename birth --choose --engine codex  --role "แรงงานโค้ด"
codename birth lyra --engine claude --role "นักวิจัย"     # ตั้งชื่อให้เอง
```

- `--choose` เปิดที่นั่งชั่วคราวแล้วให้ทารกเลือกชื่อเองพร้อมเหตุผล (ชื่อชนคำสั่งหรือ codename อื่น → ถามใหม่ สูงสุด 3 รอบ)
- บ้านอยู่ที่ `<root>\codenames\<ชื่อ>` เป็น git repo ใหม่ มี `SOUL.md`, `CLAUDE.md`, `AGENTS.md`, `BIRTH.md`, `ψ\…` และ config MCP ของ arra แบบเฉพาะโปรเจกต์
- หลังเกิดจะ `wakeup` แล้วถาม "แนะนำตัวสั้นๆ ว่าเป็นใคร เผ่าอะไร" — ต้องตอบชื่อถูก บอกเผ่า codename และลงท้ายค่ะ (ข้ามด้วย `--no-verify`, ไม่เปิดที่นั่งด้วย `--no-wake`)
- บันทึกการเกิดต่อท้าย `%USERPROFILE%\.codename\REGISTRY-local.md`

## ใช้งานประจำวัน

```powershell
codename list                                   # ตารางกองเรือ (ls)
codename wakeup lyra                            # เปิดที่นั่ง (wake) — ถ้าเคยตื่นแล้วจะคุยต่อจากรอบก่อน
codename wakeup lyra --fresh                    # เริ่มบทสนทนาใหม่
codename hey lyra "ช่วยดู README หน่อยค่ะ"        # ส่งแล้วไม่รอ
codename ask vela "ตอบคำเดียว: CN-OK"            # ส่ง รอ พิมพ์คำตอบล่าสุด
codename wait vela --timeout 900                # 0=เสร็จ 2=หมดเวลา 3=ติดค้างรอคน
codename peek vela -n 60                        # ดูหน้าจอ
codename wt vela fix-login                      # worktree แยกสำหรับงานโค้ด
codename sleep vela                             # ปิดที่นั่ง (rest)
```

จากในที่นั่งของ codename เอง (cwd อยู่ในบ้าน):

```powershell
codename report "แก้ bug login เสร็จ commit abc123"     # → REPORT vela rc=0: ...
codename report --rc 1 "เทสต์ยังไม่ผ่าน 2 ข้อ"
codename remember "Orca ต้องใส่ --from ทุกคำสั่ง orchestration"
codename recall "--from"
```

## ทีม

```powershell
codename team up examples\charter.yaml
codename team status
codename team down demo
```

charter: `name`, `repo` (ชื่อ codename หรือ path ของ git repo), `lead` (ไม่บังคับ), `workers` (`name`, `engine`, `task`) — ดู `examples/charter.yaml` · ทุกคำสั่ง orchestration ใส่ `--from <terminal กัปตัน>` ให้อัตโนมัติ · `team down` ปิด terminal, commit WIP ที่ยังไม่ commit ลง branch ของ worktree แล้วค่อยลบ worktree (ถ้าหา path ไม่เจอจะ **เก็บไว้** และ exit 1)

## หลังรีบูต

```powershell
codename startup     # (up) เช็ก Orca + arra, เปิด terminal กัปตัน, ปลุกทุกตัวที่ควรตื่น
codename shutdown    # (down) พักทุกตัว ลูกทีมก่อน กัปตันหลังสุด — ไม่ปิดแอป Orca
```

ทำให้รันเองตอน login: Task Scheduler → "At log on" → `codename startup`

### runtime ของ Orca แบบไม่ต้องเปิดแอป (serve-task)

```powershell
codename serve-task install     # (svc install) สร้าง task "codename-orca-serve": ตอน logon รัน orca serve --no-pairing --json
codename serve-task status      # ดูว่ามี task ไหม
codename startup                # ถ้า runtime ไม่ตอบ จะสั่ง task นี้ (หรือ orca open ถ้าไม่ได้ตั้ง task) แล้วรอ 60 วิ
codename serve-task remove      # ลบ task
```

- task รันในเซสชันของผู้ใช้ (ไม่ต้องใช้ admin โดยปกติ) — `orca serve` ที่สั่งผ่าน SSH จะตายเมื่อ SSH หลุด
- `orca serve` เปิด websocket ที่ **0.0.0.0:6768** — **ห้ามเปิดพอร์ตนี้ใน Windows Firewall** · codename ใส่ `--no-pairing` ให้เสมอ
- `codename doctor` บอกสถานะ runtime (`orca status`) และว่าโฮสต์โดยแอปหรือ serve

## โหมดฟรี (Free Claude Code)

โหมดฟรีให้ codename ทำงานบนโมเดลฟรี (เช่น NVIDIA NIM) ผ่านเครื่องมือภายนอกชื่อ **Free Claude Code (FCC)** — https://github.com/Alishahryar1/free-claude-code · เป็นทางเลือกเสริม ไม่บังคับค่ะ · FCC ใช้สัญญาอนุญาต AGPL-3.0 จึง**ไม่ได้รวมมากับ codename** (codename เป็น MIT) ต้องติดตั้งแยกเอง codename แค่เรียกตัวเปิดที่ติดตั้งแล้ว (`fcc-claude`, `fcc-codex`) เท่านั้นค่ะ

**ติดตั้ง FCC (ทำเองใน PowerShell)**

```powershell
& ([scriptblock]::Create((irm "https://raw.githubusercontent.com/Alishahryar1/free-claude-code/main/scripts/install.ps1")))
```

- ตอนตัวติดตั้งถามว่าจะลง agent ที่มีอยู่แล้ว (claude, codex) ไหม ให้ตอบ **NO** ค่ะ ไม่งั้นจะมีอีกชุดหนึ่งไปบังตัวเดิมใน PATH
- เปิด Admin UI ของ FCC → ใส่ NVIDIA NIM key (สร้างที่ build.nvidia.com/settings/api-keys) → กด Apply
- ตัวเปิดจะอยู่ใน `%USERPROFILE%\.local\bin` (`fcc-claude.exe`, `fcc-codex.exe`, …) และ config อยู่ที่ `~/.fcc/.env`

**ลงทะเบียน engine แล้วให้กำเนิด codename**

```powershell
codename setup --free
codename birth lyra --engine claude-free --role "ผู้ช่วยงานเบา"
codename birth --choose --engine codex-free --role "แรงงานโค้ด" --no-verify
```

- `setup --free` เพิ่ม `claude-free` (→ `fcc-claude`, คุยต่อด้วย `fcc-claude --continue`) และ `codex-free` (→ `fcc-codex`, คุยต่อด้วย `fcc-codex resume --last`) ลง `engines.json` · ถ้ามี key นี้อยู่แล้วจะไม่แตะของเดิมเลยค่ะ (แก้เองได้) · รันซ้ำได้ ไม่เขียนไฟล์ถ้าไม่มีอะไรใหม่
- ถ้ายังไม่ได้ติดตั้ง FCC หรือ config ยังไม่ปลอดภัย `setup --free` และ `codename doctor` (หัวข้อ `free mode`) จะบอกค่ะ

**ข้อควรรู้เรื่องคุณภาพ**

- โมเดลฟรีทำตามบุคลิก/ภาษาไทยได้แย่กว่า อาจตอบไม่ลงท้ายค่ะหรือไม่บอกเผ่า codename ทำให้การตรวจแนะนำตัวหลังเกิดไม่ผ่าน — ใช้ `--no-verify` ได้ค่ะ
- มีขีดจำกัดจำนวนคำขอ เช่น NVIDIA NIM ประมาณ 40 ครั้ง/นาที — อย่าเปิดหลายที่นั่งพร้อมกันเยอะเกินไป
- codename ที่เกิดด้วย `claude-free`/`codex-free` ได้ความจำกลาง arra (MCP) เหมือนตัวปกติค่ะ
- ยังใช้เป็นลูกทีมใน `team up` ไม่ได้ เพราะ Orca เปิดลูกทีมด้วย agent ของมันเอง (จะได้ claude/codex ตัวจริง ไม่ผ่าน FCC) — ใช้ `birth`/`hey` แทนค่ะ
- ถ้า FCC ไม่ทำงาน (proxy ล่ม, ยังไม่ใส่ key, หา claude/codex ไม่เจอ) ตอนเกิดหรือตื่น codename จะหยุดแล้วบอกให้เปิด Admin UI / รัน `fcc-doctor` — **ไม่**ได้บอกให้ `/login` เพราะโหมดฟรีไม่ควรไปล็อกอินบัญชีจริงค่ะ

**ความปลอดภัย**

- ตั้ง **Host เป็น 127.0.0.1** และเปิด **Proxy Authentication** ใน Admin UI ของ FCC (ค่าเริ่มต้นของ FCC คือ HOST=0.0.0.0, PORT=8082 และปิด auth — เครื่องอื่นใน LAN/tailnet จะใช้ key ของเราได้) · `doctor` เตือนให้ถ้ายังไม่ได้ตั้ง และไม่เคยพิมพ์ค่า token หรือ API key ออกมา
- อย่ากด Admin UI → Integrations → Connect ให้ Claude Desktop / VS Code ถ้าไม่ได้ตั้งใจให้แอปพวกนั้นวิ่งผ่าน FCC ด้วยค่ะ
- codename **ไม่แตะ `ANTHROPIC_*` เลย** (ไม่ตั้งใน env ของผู้ใช้, `settings.json` หรือ `engines.json`) และไม่รันตัวติดตั้ง ไม่เขียน `~/.fcc/.env` — ตัวเปิดของ FCC ตั้งค่าเฉพาะโปรเซสของมันเอง
- ถ้าหน้าจอขึ้น "Detected a custom API key / Do you want to use this API key?" หรือหน้าเลือก Anthropic Console ตอนเกิดหรือตื่น codename จะหยุดแล้วรายงานว่า `blocked` โดยไม่ตอบให้ค่ะ — ให้พี่เก่งตัดสินใจเอง

**ระวังลิงก์ผิด:** https://github.com/Rishurajgautam24/free-claude-code ไม่ใช่ fork แต่เป็นการอัปโหลดซ้ำแบบเก่าที่หยุดไว้ตั้งแต่ 2026-03-05 (รุ่นยุค MIT ที่รันด้วย uvicorn ไม่มีตัวเปิด `fcc-*` และไม่มีตัวติดตั้งสำหรับ Windows) — ใช้ต้นทางของ Alishahryar1 ข้างบนเท่านั้นค่ะ

## ตั้งค่า

| ไฟล์ (ใน `%USERPROFILE%\.codename\`) | ใช้ทำอะไร |
|---|---|
| `engines.json` | engine → คำสั่งเปิด แบบสั้น `{"glm":"claude-glm"}` หรือแบบเต็ม `{"codex":{"command":"codex","resume_command":"codex resume --last"}}` · `resume_command` ใช้ตอน `wakeup` ครั้งถัดไป/`startup` (ค่าเริ่มต้น claude → `claude --continue`, codex → `codex resume --last` ยังไม่ได้ตรวจ) · แก้ได้ไม่ต้องคอมไพล์ใหม่ |
| `config.json` | `root`, `arra_url`, path ของ API arra, `arra_mcp_url`, `orca_bin` (path เต็มของ Orca CLI `.exe`), `serve_task` (ชื่อ scheduled task), `terminal_shell` (เช่น `pwsh.exe` — ส่งเป็น `--shell` ตอนเปิดที่นั่ง) · `engines.json` ใส่ `orca_agent` ได้ (agent id ของ Orca สำหรับ `team`: claude, codex, cursor, antigravity, …) |
| `logs\` | log JSON-lines รายวัน เก็บ 14 วัน ไฟล์เก่าอยู่ใน `logs\old\` · ใส่ `--verbose` เพื่อดูออก stderr ด้วย |
| `templates\` | วางไฟล์ `SOUL.md` / `CLAUDE.md` / `AGENTS.md` / `BIRTH.md` / `RULES.md` เพื่อทับ template ที่ฝังในโปรแกรม |
| env `CODENAME_HOME` | ย้ายโฟลเดอร์ `.codename` ไปที่อื่น |
| env `CODENAME_ORCA_BIN` | path ของ Orca CLI (ชนะ `orca_bin` ใน config) — ต้องเป็น `.exe` ไม่ใช่ `.cmd`/`.bat` |
| env `CODENAME_NAME` | ชื่อผู้ส่งสำหรับ `report`/`remember` ถ้าไม่ได้รันในบ้านของ codename ค่ะ |

## ข้อควรระวัง

- **ต้องมี runtime ของ Orca** (แอปเปิดอยู่ หรือ `orca serve` จาก serve-task) ไม่งั้นได้ error `runtime_unavailable` — แก้ด้วย `codename startup`
- **กับดัก `--from`**: orchestration ของ Orca ต้องมี terminal ผู้ส่ง — ถ้า terminal กัปตันหาย ให้ `codename startup` หรือ `codename setup`
- **ห้ามให้ Orca เปิด agent ซ้ำ** กับที่นั่งที่ codename คุม (เช่นกดเปิด agent เองใน UI ของ Orca ในบ้านเดียวกัน) — `doctor` จะเตือน "started twice?"
- **hook ที่ Orca ใส่ใน config ของ agent**: `doctor` บอกไฟล์ที่เปลี่ยนจาก baseline — ไฟล์สำรองอยู่ใน `backups\pre-orca-*\files\` ตัดสินใจเองว่าจะเก็บหรือถอด
- `wakeup` ตอบหน้าต่าง "trust this folder" ให้เท่านั้น — หน้าต่างล็อกอิน/OAuth และหน้าต่างทบทวน hook จะหยุดให้มนุษย์ทำเอง
- **orca ที่เป็น `.cmd`/`.bat` ถูกปฏิเสธ** (กันการแทรกคำสั่งผ่าน `cmd.exe`, CVE-2024-24576) — ดู path ที่เจอด้วย `codename doctor` แล้วตั้ง `CODENAME_ORCA_BIN` ไปที่ `.exe`
- **อย่าใช้โฟลเดอร์ OneDrive** เป็น root หรือบ้าน codename — `setup`/`birth`/`doctor` เตือนให้ · path ที่มีภาษาไทยหรือช่องว่างใช้ได้ แต่จะมีข้อความแจ้งไว้
- flag ของ Orca ตรวจกับ Orca 1.4.224 แล้ว — ถ้า Orca รุ่นใหม่เปลี่ยน ดู `docs/orca-cli-observed.md` และแก้ตารางหัวไฟล์ `src/orca.rs`
