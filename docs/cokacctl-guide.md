# cokacctl 사용 가이드

cokacdir의 설치, 업데이트, 백그라운드 서비스를 통합 관리하는 도구입니다.

---

## 명령어 요약

```
cokacctl                         TUI 대시보드 실행
cokacctl install                 cokacdir 바이너리 설치
cokacctl update                  cokacdir 최신 버전으로 업데이트
cokacctl status                  플랫폼/버전/서비스 상태 표시
cokacctl token <TOKEN>           봇 토큰 등록/변경
cokacctl start                   서비스 등록 및 시작
cokacctl stop                    서비스 중지
cokacctl restart                 서비스 재시작
cokacctl remove                  서비스 삭제
cokacctl log                     로그 실시간 확인 (tail -f)
```

> `<TOKEN>`은 텔레그램 봇 토큰입니다. 여러 개를 공백으로 구분하여 전달할 수 있습니다.

---

## macOS

### 필수 환경

- macOS 10.15 이상
- launchd (기본 내장)

### 1. cokacdir 설치

```bash
cokacctl install
```

- `/usr/local/bin/cokacdir`에 바이너리를 다운로드합니다.
- 권한이 없으면 자동으로 `sudo`를 사용합니다. `sudo`도 실패하면 `~/.local/bin/cokacdir`에 설치합니다.
- `~/.zshrc`에 shell wrapper 함수를 자동 추가합니다:
  ```bash
  cokacdir() { command cokacdir "$@" && cd "$(cat ~/.cokacdir/lastdir 2>/dev/null || pwd)"; }
  ```

### 2. 토큰 등록

```bash
cokacctl token YOUR_BOT_TOKEN
```

### 3. 서비스 시작

```bash
cokacctl start
```

내부 동작:
- `~/Library/Logs/cokacdir/run.sh` 래퍼 스크립트 생성
- `~/Library/LaunchAgents/com.cokacdir.server.plist` 생성
- `launchctl bootstrap` + `launchctl enable`로 서비스 등록
- `RunAtLoad`, `KeepAlive` 활성화 — 로그인 시 자동 시작, 비정상 종료 시 자동 재시작

### 4. 서비스 관리

```bash
cokacctl status      # 상태 확인
cokacctl stop        # 중지 (launchctl bootout)
cokacctl restart     # 재시작
cokacctl remove      # plist 파일 삭제 및 서비스 해제
cokacctl log         # ~/Library/Logs/cokacdir/cokacdir.log 실시간 출력
```

### 5. 토큰 변경

```bash
cokacctl token NEW_TOKEN_1 NEW_TOKEN_2
```

토큰을 변경한 뒤 실행 중인 서비스에 반영하려면 `cokacctl restart`를 실행하세요.

### 6. 업데이트

```bash
cokacctl update
```

- 서비스가 실행 중이면 자동으로 중지 → 업데이트 → 재시작합니다.
- `/usr/local/bin` 쓰기 권한이 없으면 `sudo`를 사용합니다.

### macOS 관련 파일 경로

```
바이너리           /usr/local/bin/cokacdir
서비스 정의        ~/Library/LaunchAgents/com.cokacdir.server.plist
래퍼 스크립트      ~/Library/Logs/cokacdir/run.sh
로그              ~/Library/Logs/cokacdir/cokacdir.log
에러 로그         ~/Library/Logs/cokacdir/cokacdir.error.log
설정 파일         ~/.cokacdir/cokacctl.json
```

---

## Linux

### 필수 환경

- systemd 기반 Linux 배포판
- `systemctl`, `loginctl` 명령어 사용 가능

### 1. cokacdir 설치

```bash
cokacctl install
```

- `/usr/local/bin/cokacdir`에 설치합니다.
- 권한 없으면 `sudo` → 실패 시 `~/.local/bin/cokacdir` 순으로 시도합니다.
- `~/.bashrc`(또는 `~/.bash_profile`)에 shell wrapper 함수를 추가합니다.

### 2. 토큰 등록

```bash
cokacctl token YOUR_BOT_TOKEN
```

### 3. 서비스 시작

```bash
cokacctl start
```

내부 동작:
- `~/.local/state/cokacdir/run.sh` 래퍼 스크립트 생성
- `~/.config/systemd/user/cokacdir.service` 유닛 파일 생성
- `systemctl --user daemon-reload` → `enable` → `restart`
- `loginctl enable-linger $USER` 실행 — 로그아웃 후에도 서비스 유지

systemd 버전에 따라 로그 출력 방식이 자동 결정됩니다:
- v240+ : `append:` (파일에 추가)
- v236+ : `file:` (파일에 쓰기)
- 그 이하 : `journal` (journald)

### 4. 서비스 관리

```bash
cokacctl status      # systemctl --user is-active cokacdir
cokacctl stop        # systemctl --user stop cokacdir
cokacctl restart     # stop → start
cokacctl remove      # stop → disable → 파일 삭제 → daemon-reload
cokacctl log         # ~/.local/state/cokacdir/cokacdir.log 실시간 출력
```

### 5. 토큰 변경

```bash
cokacctl token NEW_TOKEN_1 NEW_TOKEN_2
```

토큰을 변경한 뒤 실행 중인 서비스에 반영하려면 `cokacctl restart`를 실행하세요.

### 6. 업데이트

```bash
cokacctl update
```

### Linux 관련 파일 경로

```
바이너리           /usr/local/bin/cokacdir
서비스 정의        ~/.config/systemd/user/cokacdir.service
래퍼 스크립트      ~/.local/state/cokacdir/run.sh
로그              ~/.local/state/cokacdir/cokacdir.log
에러 로그         ~/.local/state/cokacdir/cokacdir.error.log
설정 파일         ~/.cokacdir/cokacctl.json
```

> `XDG_STATE_HOME`이 설정되어 있으면 `~/.local/state` 대신 해당 경로를 사용합니다.

---

## Windows

### 필수 환경

- Windows 10 이상
- PowerShell 5.1 이상 (기본 내장)
- 관리자 권한 (Task Scheduler 등록 시 필요)

### 1. cokacdir 설치

```powershell
cokacctl install
```

- `%USERPROFILE%\cokacdir.exe`에 바이너리를 다운로드합니다.
- Windows에서는 shell wrapper를 설정하지 않습니다.

### 2. 토큰 등록

```powershell
cokacctl token YOUR_BOT_TOKEN
```

### 3. 서비스 시작

```powershell
cokacctl start
```

내부 동작:
- Windows Task Scheduler에 `cokacdir` 태스크를 등록합니다.
- PowerShell을 통해 실행:
  - `New-ScheduledTaskAction` — 실행 바이너리 및 인자 설정
  - `New-ScheduledTaskTrigger -AtLogon` — 로그인 시 자동 시작
  - `Register-ScheduledTask -RunLevel Highest` — 관리자 권한으로 실행
  - `Start-ScheduledTask` — 즉시 시작
- 작업 디렉토리는 `%USERPROFILE%`로 설정됩니다.

### 4. 서비스 관리

```powershell
cokacctl status      # Get-ScheduledTask로 상태 조회
cokacctl stop        # Stop-ScheduledTask
cokacctl restart     # stop → start
cokacctl remove      # Unregister-ScheduledTask
cokacctl log         # %USERPROFILE%\.cokacdir\logs\cokacdir.log 실시간 출력
```

### 5. 토큰 변경

```powershell
cokacctl token NEW_TOKEN_1 NEW_TOKEN_2
```

토큰을 변경한 뒤 실행 중인 서비스에 반영하려면 `cokacctl restart`를 실행하세요.

### 6. 업데이트

```powershell
cokacctl update
```

### Windows 관련 파일 경로

```
바이너리           %USERPROFILE%\cokacdir.exe
서비스 정의        Task Scheduler → "cokacdir" 태스크
로그              %USERPROFILE%\.cokacdir\logs\cokacdir.log
에러 로그         %USERPROFILE%\.cokacdir\logs\cokacdir.error.log
설정 파일         %USERPROFILE%\.cokacdir\cokacctl.json
```

---

## TUI 대시보드

서브커맨드 없이 `cokacctl`만 실행하면 TUI 대시보드에 진입합니다.

```bash
cokacctl
```

### 화면 구성

- **버전 패널** — cokacdir/cokacctl 버전, 업데이트 가능 여부
- **서비스 패널** — 서비스 상태(Running/Stopped/Not installed), 등록된 토큰 수
- **로그 패널** — 서비스 로그 실시간 표시

### 키보드 단축키

```
Q           종료
L           로그 전체화면 (Esc 또는 L로 복귀)
S           서비스 시작 (사전에 CLI로 토큰 등록 필요)
T           서비스 중지
R           서비스 재시작
D           서비스 삭제
K           토큰 관리
U           업데이트 안내
I           설치 안내
P           바이너리 경로 설정
Ctrl+C      강제 종료
```

> TUI에서 서비스를 시작하려면 먼저 `cokacctl token <TOKEN>`으로 토큰을 등록해야 합니다.
> 이후에는 TUI에서 `S` 키로 시작/`T` 키로 중지할 수 있습니다.

### 자동 갱신

- 서비스 상태: 약 5초마다 자동 갱신
- 로그: 약 2초마다 새 내용 자동 반영
- 업데이트 확인: TUI 진입 시 백그라운드에서 1회 수행

---

## 상태 확인

```bash
cokacctl status
```

출력 예시:

```
  Platform:  linux/x86_64
  cokacctl:  v0.1.0
  cokacdir:  v0.4.67 (/usr/local/bin/cokacdir)
  Service:   ● Running  (systemd)
  Tokens:    2 bot(s)
  Log:       /home/user/.local/state/cokacdir/cokacdir.log
```

---

## 설정 파일

토큰, 비활성화된 토큰 목록, 설치 경로는 `~/.cokacdir/cokacctl.json`에 저장됩니다.

```json
{
  "tokens": ["123456:ABC-DEF..."],
  "disabled_tokens": [],
  "install_path": "/usr/local/bin/cokacdir"
}
```

- Unix에서 파일 권한은 `0600`으로 자동 설정됩니다.
- 이 파일은 `cokacctl token ...` 실행 시 자동 생성/갱신되며, TUI에서 토큰을 저장하거나 바이너리 경로를 저장해도 갱신됩니다.
- 수동으로 편집하지 않아도 됩니다.

---

## 전체 워크플로우 (처음부터 끝까지)

```bash
# 1. cokacdir 설치
cokacctl install

# 2. 설치 확인
cokacctl status

# 3. 봇 토큰 등록
cokacctl token YOUR_BOT_TOKEN

# 4. 백그라운드 서비스 시작
cokacctl start

# 5. 서비스 상태 확인
cokacctl status

# 6. 로그 확인
cokacctl log

# 7. 새 버전이 나오면 업데이트
cokacctl update

# 8. 토큰 변경이 필요하면
cokacctl token NEW_TOKEN
cokacctl restart

# 9. 서비스 완전 제거
cokacctl remove
```
