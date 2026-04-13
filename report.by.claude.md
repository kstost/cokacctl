# cokacctl 변경사항 검사 보고서

- 검사일: 2026-04-13
- 비교 대상: `cokacctl` (현재) vs `cokacctl_old` (이전)
- 검사 도구: Claude Opus 4.6

---

## 변경 개요

변경된 파일 3개:

| 파일 | 현재 버전 (라인) | 이전 버전 (라인) | 변경 방향 |
|------|-----------------|-----------------|----------|
| `src/core/platform.rs` | 305 | 281 | WindowsServiceState 구조체 및 state_file 추가 |
| `src/service/taskscheduler.rs` | 479 | 354 | 프로세스 기반 → Task Scheduler 상태 기반으로 전면 재설계 |
| `src/cli/uninstall.rs` | 226 | 227 | Windows 언인스톨 절차 변경 |

핵심 변경 방향: 기존의 **프로세스 기반(PID 파일 + tasklist 조회) 단순 접근법**에서 **Task Scheduler 상태 조회(PowerShell) + WindowsServiceState JSON 파일 접근법**으로 전환.

---

## 검사 결과 요약

| # | 항목 | 심각도 | 상태 |
|---|------|--------|------|
| 1 | uninstall.rs: Windows 프로세스 강제 종료 누락 | **높음** | 수정 필요 |
| 2 | taskscheduler.rs: stop/remove 에러 전파로 인한 연쇄 실패 | **중간** | 수정 고려 |
| 3 | taskscheduler.rs: is_any_running() trait 의미 불일치 | **중간** | 수정 필요 |
| 4 | platform.rs: WindowsServiceState 추가 | 없음 | 올바름 |
| 5 | taskscheduler.rs: cmd.exe /c 방식 변경 | 없음 | 개선됨 |
| 6 | taskscheduler.rs: wait_for_running() 정교한 검증 | 없음 | 올바름 |
| 7 | taskscheduler.rs: status() PowerShell 방식 | 없음 | 올바름 |
| 8 | uninstall.rs: windows-service.json 삭제 목록 추가 | 없음 | 올바름 |

---

## 상세 분석

---

### 1. [높음] uninstall.rs - Windows에서 프로세스 강제 종료가 제거됨

**파일:** `src/cli/uninstall.rs` 113~138행

#### 변경 전 (cokacctl_old)

```rust
// 1단계: 태스크 삭제
let mut cmd = Command::new("schtasks");
cmd.args(["/Delete", "/TN", "cokacdir", "/F"]);
// ...

// 2단계: 모든 cokacdir* 프로세스 강제 종료
let mut ps = Command::new("powershell");
ps.args(["-NoProfile", "-NonInteractive", "-Command",
    "Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' } | ForEach-Object { Write-Output \"Killing PID=$($_.Id) Name=$($_.ProcessName)\"; Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }"]);
```

#### 변경 후 (cokacctl)

```rust
// 1단계: 태스크 중지
let mut ps = Command::new("powershell");
ps.args(["-NoProfile", "-NonInteractive", "-Command",
    "Stop-ScheduledTask -TaskName 'cokacdir' -ErrorAction SilentlyContinue"]);
// ...

// 2단계: 태스크 삭제
let mut cmd = Command::new("schtasks");
cmd.args(["/Delete", "/TN", "cokacdir", "/F"]);
// ...

// !! 프로세스 강제 종료 단계가 없음 !!
```

#### 문제점

현재 버전은 `Stop-ScheduledTask`에만 의존하여 프로세스 종료를 처리한다. 그러나:

1. **Task Scheduler 외부에서 수동 실행된 cokacdir 프로세스는 종료되지 않음.** 사용자가 커맨드라인에서 직접 cokacdir를 실행한 경우, `Stop-ScheduledTask`는 해당 프로세스에 영향을 주지 않는다.

2. **`Stop-ScheduledTask`가 자식 프로세스를 확실히 종료하지 않을 수 있음.** Task Scheduler의 프로세스 종료 방식은 graceful shutdown이며, cokacdir가 종료 신호를 즉시 처리하지 않으면 프로세스가 잔존할 수 있다.

3. **플랫폼 간 동작이 비일관적.** MacOS와 Linux의 uninstall 코드는 여전히 서비스 중지 후 `pkill cokacdir`로 모든 프로세스를 명시적으로 종료한다:
   - MacOS (`uninstall.rs:78-86`): `Command::new("pkill").arg("cokacdir")`
   - Linux (`uninstall.rs:102-110`): `Command::new("pkill").arg("cokacdir")`
   - Windows: **명시적 프로세스 종료 없음**

#### 영향

uninstall 완료 후에도 cokacdir 프로세스가 고아 프로세스로 살아남을 수 있다. 이후 파일 삭제 단계에서 바이너리 파일(`cokacdir.exe`)이 프로세스에 의해 잠겨있어 삭제 실패할 수 있다.

#### 권장 수정

`schtasks /Delete` 이후에 이전 버전의 프로세스 Kill 로직을 추가:

```rust
// schtasks /Delete 이후 추가
dlog!("uninstall", "Killing all cokacdir* processes via PowerShell...");
let mut ps_kill = Command::new("powershell");
ps_kill.args(["-NoProfile", "-NonInteractive", "-Command",
    "Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' } | ForEach-Object { Write-Output \"Killing PID=$($_.Id) Name=$($_.ProcessName)\"; Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }"]);
ps_kill.creation_flags(0x08000000);
match ps_kill.output() {
    Ok(out) => {
        let stdout = String::from_utf8_lossy(&out.stdout);
        dlog!("uninstall", "kill cokacdir* exit={}, stdout='{}'", out.status, stdout.trim());
    }
    Err(e) => {
        dlog!("uninstall", "kill cokacdir* failed: {}", e);
    }
}
```

---

### 2. [중간] taskscheduler.rs - stop()/remove() 에러 전파로 인한 연쇄 실패

**파일:** `src/service/taskscheduler.rs`

#### 변경 전 (cokacctl_old)

```rust
// stop() - 항상 Ok(()) 반환
fn stop(&self) -> Result<(), String> {
    let stop_result = Self::powershell(...);          // 결과 무시
    if let Ok(ref out) = stop_result { /* 로그만 */ }
    let kill_result = Self::powershell(...);           // 결과 무시
    match kill_result { /* 로그만 */ }
    self.clear_pid();
    Ok(())  // 항상 성공
}

// remove() - 항상 Ok(()) 반환
fn remove(&self) -> Result<(), String> {
    let _ = self.stop();                               // 에러 무시
    let del_result = Self::cmd("schtasks")...;         // 결과 무시
    if let Ok(ref out) = del_result { /* 로그만 */ }
    std::fs::remove_file(...).ok();                    // 에러 무시
    Ok(())  // 항상 성공
}

// start() - remove 에러 무시
fn start(...) -> Result<(), String> {
    let remove_result = self.remove();                 // 에러 무시
    dlog!("taskscheduler", "remove result: {:?}", remove_result);
    // ... 계속 진행 ...
}
```

#### 변경 후 (cokacctl)

```rust
// stop() - 실패 전파 가능
fn stop(&self) -> Result<(), String> {
    self.stop_task_if_present()?;   // PowerShell 실패 시 에러 전파
    self.kill_cokacdir_processes();
    self.clear_legacy_pid_file();
    Ok(())
}

// remove() - 4개 실패점
fn remove(&self) -> Result<(), String> {
    self.stop_task_if_present()?;                      // 실패점 1
    self.kill_cokacdir_processes();
    self.delete_task_if_present()?;                    // 실패점 2
    if self.paths.wrapper_script.exists() {
        std::fs::remove_file(&self.paths.wrapper_script)
            .map_err(...)?;                            // 실패점 3
    }
    self.remove_state_file()?;                         // 실패점 4
    self.clear_legacy_pid_file();
    Ok(())
}

// start() - remove 에러 전파
fn start(...) -> Result<(), String> {
    self.remove()?;  // remove 실패 → start 전체 실패
    // ...
}
```

#### 문제점: 연쇄 실패 경로

```
PowerShell 실행 실패 또는 Task Scheduler 응답 없음
  → query_task_state() 실패
    → stop_task_if_present() 실패
      → stop() 실패
        → remove() 실패
          → start() 실패
```

**구체적 시나리오:**

1. PowerShell이 일시적으로 응답하지 않는 경우 (Windows Update 직후 등)
2. Task Scheduler 서비스가 비정상 상태인 경우
3. 다른 사용자 계정의 동일 이름 태스크가 존재하여 쿼리는 되나 Stop 권한이 없는 경우

이전 버전에서는 이런 상황에서도 best-effort로 진행하여 서비스를 시작할 수 있었지만, 현재 버전에서는 cleanup 단계의 실패가 전체 시작을 차단한다.

#### 영향

사용자가 서비스를 시작/재시작하려 할 때, 이전 상태 정리 실패로 인해 시작 자체가 불가능해질 수 있다. 이전 버전에서는 발생하지 않던 실패 시나리오가 새로 생김.

#### 권장 수정

`start()` 내의 `self.remove()?`를 이전 버전처럼 에러를 무시하도록 변경:

```rust
// 방법 A: remove 에러 무시 (이전 버전과 동일)
let remove_result = self.remove();
dlog!("taskscheduler", "remove result: {:?}", remove_result);

// 방법 B: remove 내부에서 개별 단계 에러를 무시
fn remove(&self) -> Result<(), String> {
    let _ = self.stop_task_if_present();   // 에러 무시
    self.kill_cokacdir_processes();
    let _ = self.delete_task_if_present(); // 에러 무시
    // ...
    Ok(())
}
```

---

### 3. [중간] taskscheduler.rs - is_any_running()이 trait 정의와 불일치

**파일:** `src/service/taskscheduler.rs` 471행

#### trait 정의 (service/mod.rs:43-44)

```rust
/// Check if any cokacdir process is running externally (regardless of service manager).
fn is_any_running(&self) -> bool;
```

주석이 명확히 "서비스 매니저와 무관하게" 프로세스 실행 여부를 확인하라고 요구한다.

#### 변경 전 (cokacctl_old) - trait 정의에 부합

```rust
fn is_any_running(&self) -> bool {
    Self::is_cokacdir_running()  // tasklist에서 실제 cokacdir* 프로세스 검색
}
```

`tasklist` 명령으로 시스템의 모든 cokacdir* 프로세스를 검색한다. Task Scheduler로 시작했든, 수동으로 시작했든, 어떤 방식이든 상관없이 프로세스 존재 여부를 확인한다.

#### 변경 후 (cokacctl) - trait 정의와 불일치

```rust
fn is_any_running(&self) -> bool {
    self.status() == ServiceStatus::Running  // Task Scheduler 상태만 확인
}
```

`self.status()`는 `query_task_state()`를 호출하며, 이는 PowerShell로 `Get-ScheduledTask`의 State 속성만 확인한다. **Task Scheduler에 등록되지 않은 프로세스는 감지하지 못한다.**

#### 호출 위치 및 영향

| 파일 | 행 | 용도 | 영향 |
|------|---|------|------|
| `src/cli/install.rs` | 49 | 설치 전 실행 중인 프로세스 확인 | 수동 실행 프로세스 미감지 → 바이너리 교체 시 파일 잠김 가능 |
| `src/cli/update.rs` | 63 | 업데이트 전 실행 중인 프로세스 확인 | 수동 실행 프로세스 미감지 → 바이너리 교체 시 파일 잠김 가능 |
| `src/tui/event.rs` | 384 | TUI에서 시작 전 기존 프로세스 확인 | 수동 실행 프로세스 미감지 → 중복 실행 가능 |

#### 참고: 다른 플랫폼의 구현

MacOS (`launchd.rs:264-276`)와 Linux (`systemd.rs:311-323`)의 `is_any_running()`은 모두 `pgrep cokacdir`로 실제 프로세스를 확인한다. 현재 Windows 구현만 이 패턴에서 벗어나 있다.

#### 권장 수정

이전 버전의 구현으로 되돌리기:

```rust
fn is_any_running(&self) -> bool {
    Self::is_cokacdir_running()
}
```

현재 버전에도 `is_cokacdir_running()` 함수 자체는 그대로 존재하므로(63-85행), 호출만 변경하면 된다.

---

### 4. [문제 없음] platform.rs - WindowsServiceState 추가

**파일:** `src/core/platform.rs`

#### 추가된 항목

1. `use serde::{Deserialize, Serialize};` (1행)
2. `ServicePaths` 구조체에 `state_file: PathBuf` 필드 추가 (162행)
3. `WindowsServiceState` 구조체 추가 (168-175행)
4. `windows_service_state()` 메서드 추가 (256-259행)
5. `running_token_count()`에 Windows 전용 분기 추가 (224-228행)

#### 검증

- **serde 의존성:** `Cargo.toml`에 `serde = { version = "1", features = ["derive"] }`와 `serde_json = "1"` 이미 존재. 양쪽 버전 동일. 문제 없음.

- **state_file 경로:**
  - MacOS: `~/Library/Logs/cokacdir/service-state.json` (사용되지 않으나 해 없음)
  - Linux: `~/.local/state/cokacdir/service-state.json` (사용되지 않으나 해 없음)
  - Windows: `~/.cokacdir/windows-service.json` (taskscheduler.rs에서 사용)

- **running_token_count() 로직:**
  ```rust
  if cfg!(windows) {
      if let Some(state) = self.windows_service_state() {
          return Some(state.token_count);
      }
  }
  // fallback: wrapper script 파싱
  ```
  JSON 파일을 우선 확인하고, 실패 시 기존 wrapper script 파싱으로 fallback. 합리적인 설계.

- **호출 위치:** `tui/app.rs`에서 `running_token_count()` 호출 (78-79행, 129-130행). 기존 호출 코드 변경 불필요.

- **uninstall.rs 정합성:** `collect_paths()`에 `home.join(".cokacdir/windows-service.json")` 추가됨 (218행). 생성하는 파일을 삭제 목록에 포함한 것은 올바름.

**결론:** 올바르게 구현됨. 추가 수정 불필요.

---

### 5. [문제 없음] taskscheduler.rs - cmd.exe /c 방식 변경

**파일:** `src/service/taskscheduler.rs` 369-374행

#### 변경 전

```rust
let script = format!(
    "$action = New-ScheduledTaskAction -Execute '{exe}' -WorkingDirectory '{wd}'\n...",
    exe = escape_ps_single(&self.paths.wrapper_script.to_string_lossy()),
    // ...
);
```

Task Scheduler가 .bat 파일을 직접 실행.

#### 변경 후

```rust
let wrapper_path = self.paths.wrapper_script.to_string_lossy();
let script = format!(
    "$action = New-ScheduledTaskAction -Execute 'cmd.exe' -Argument '/c \"{wrapper}\"' -WorkingDirectory '{wd}'\n...",
    wrapper = escape_ps_single(&wrapper_path),
    // ...
);
```

`cmd.exe /c`를 통해 .bat 파일 실행.

#### 검증

- Task Scheduler에서 .bat 파일 직접 실행 시 간헐적으로 실행 실패하는 알려진 문제가 있음
- `cmd.exe /c`로 실행하는 것이 Microsoft 권장 방식
- wrapper script의 내용(`@echo off\r\n{exe} --ccserver -- ...`)은 변경되지 않아 호환성 유지
- `cmd.exe /c`는 .bat 실행 후 종료하므로 Task Scheduler가 프로세스 수명을 올바르게 추적 가능

**결론:** 개선된 변경. 문제 없음.

---

### 6. [문제 없음] taskscheduler.rs - wait_for_running() 정교한 검증

**파일:** `src/service/taskscheduler.rs` 236-306행

#### 변경 전 (cokacctl_old)

```rust
// 단순 2초 대기 후 프로세스 확인
std::thread::sleep(std::time::Duration::from_millis(2000));
if !Self::is_cokacdir_running() {
    let err_output = std::fs::read_to_string(&self.paths.error_log_file).unwrap_or_default();
    return Err(format!("cokacdir exited immediately: {}", err_output.trim()));
}
```

#### 변경 후 (cokacctl)

```rust
fn wait_for_running(&self) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut saw_running = false;

    while std::time::Instant::now() < deadline {
        match self.query_task_state()? {
            Some(state) if state == "Running" => { saw_running = true; }
            Some(state) if saw_running && state == "Ready" => {
                // Running → Ready 전환 감지: 로그 분석으로 성공/실패 판단
                // - startup_log_indicates_success(): 성공 마커 확인
                // - benign_error_log_only(): 무해한 stderr 필터링
                // - 실패 시 에러 로그 tail 반환
            }
            Some(_) | None => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    // 타임아웃 후 fallback 판단 로직
}
```

#### 검증

- **Running → Ready 전환 감지:** 프로세스가 즉시 종료되는 경우를 정확히 포착할 수 있음. 이전 버전의 단순 2초 sleep보다 정밀함.

- **성공 마커 (`startup_log_indicates_success`):**
  ```rust
  let success_markers = ["Bot connected", "Listening for messages", "Scheduler started", "No pending updates"];
  ```
  cokacdir의 실제 로그 출력에 기반한 판단. 로그 형식이 변경되면 갱신 필요하나, 현재 시점에서는 올바름.

- **benign stderr 필터링 (`benign_error_log_only`):**
  ```rust
  lines.iter().all(|line| {
      line.starts_with("[ccserver] token #") || line.starts_with("[ccserver]")
  })
  ```
  `[ccserver]`로 시작하는 stderr는 정상 동작 중 발생하는 정보성 출력으로 취급. 합리적.

- **진단 로그 기록 (`append_diagnostic_error`):** 검증 결과를 에러 로그 파일에 타임스탬프와 함께 기록. 디버깅에 매우 유용.
  - `chrono` 의존성: `Cargo.toml`에 이미 존재 확인.

- **실패 시 cleanup:**
  ```rust
  if let Err(e) = self.wait_for_running() {
      let _ = self.delete_task_if_present();
      let _ = self.remove_state_file();
      return Err(e);
  }
  ```
  `delete_task_if_present()`가 실행 중인 태스크의 프로세스도 종료시키므로 고아 프로세스 문제 없음.

**결론:** 이전 버전보다 훨씬 정교하고 신뢰성 있는 검증. 올바르게 구현됨.

---

### 7. [문제 없음] taskscheduler.rs - status() PowerShell 방식 변경

**파일:** `src/service/taskscheduler.rs` 458-468행

#### 변경 전 (cokacctl_old)

```rust
fn status(&self) -> ServiceStatus {
    // 1. schtasks /Query로 태스크 존재 확인
    // 2. is_saved_pid_alive()로 저장된 PID 확인
    // 3. is_cokacdir_running()으로 프로세스 확인 (fallback)
    // 4. 모두 아니면 Stopped
}
```

#### 변경 후 (cokacctl)

```rust
fn status(&self) -> ServiceStatus {
    match self.query_task_state() {
        Ok(None) => ServiceStatus::NotInstalled,
        Ok(Some(state)) => match state.as_str() {
            "Running" => ServiceStatus::Running,
            "Ready" | "Queued" | "Disabled" => ServiceStatus::Stopped,
            other => ServiceStatus::Unknown(other.to_string()),
        },
        Err(e) => ServiceStatus::Unknown(e),
    }
}
```

#### 검증

- **Task 상태와 프로세스 상태의 정합성:** wrapper script(`run.bat`)은 cokacdir를 동기적으로 실행한다 (`{exe} --ccserver -- {args} >> ...`). `start` 키워드나 백그라운드 실행이 없으므로, cokacdir가 실행 중이면 Task 상태도 "Running"이고, cokacdir가 종료되면 Task 상태도 "Ready"로 돌아간다. 1:1 대응이 보장됨.

- **PowerShell vs schtasks CLI 속도:** PowerShell 호출은 schtasks CLI보다 느리다 (약 300-800ms). `status()`가 TUI 갱신 루프에서 호출되므로 체감 가능한 지연이 있을 수 있으나, 기능적 문제는 아님.

- **상태 매핑 완전성:**
  - "Running" → Running (정확)
  - "Ready" → Stopped (정확: 태스크 등록됨, 프로세스 미실행)
  - "Queued" → Stopped (정확: 실행 대기 중)
  - "Disabled" → Stopped (정확: 비활성화됨)
  - 기타 → Unknown (안전한 fallback)
  - None → NotInstalled (태스크 미등록)

**결론:** 올바르게 구현됨. 속도 차이 외 기능적 문제 없음.

---

### 8. [문제 없음] uninstall.rs - windows-service.json 삭제 목록 추가

**파일:** `src/cli/uninstall.rs` 215-225행

#### 변경 전 (cokacctl_old)

```rust
Os::Windows => (
    vec![home.join("cokacdir.exe")],
    vec![home.join(".cokacdir/logs"), home.join(".cokacdir/scripts")],
)
```

#### 변경 후 (cokacctl)

```rust
Os::Windows => (
    vec![
        home.join("cokacdir.exe"),
        home.join(".cokacdir/windows-service.json"),  // 추가
    ],
    vec![home.join(".cokacdir/logs"), home.join(".cokacdir/scripts")],
)
```

#### 검증

- 현재 버전의 `taskscheduler.rs`에서 `~/.cokacdir/windows-service.json` 파일을 생성하므로 (write_state_file), 삭제 목록에 포함하는 것은 올바름.
- 이전 버전에서 업그레이드한 경우 이 파일이 존재하지 않을 수 있으나, `exists()` 체크 후 삭제하므로 문제 없음 (150-163행).
- PID 파일(`cokacdir.pid`)은 `~/.cokacdir/logs/` 디렉토리 내에 있으므로, `logs` 디렉토리 삭제 시 함께 제거됨. 별도 목록 추가 불필요.

**결론:** 올바르게 구현됨. 추가 수정 불필요.

---

## 부록: 변경되지 않아야 할 부분 확인

아래 항목들이 변경 영향을 받지 않았는지 확인함:

| 확인 항목 | 결과 |
|----------|------|
| `service/mod.rs` (trait 정의) | 변경 없음. 정상. |
| `service/launchd.rs` (MacOS) | 변경 없음. 정상. |
| `service/systemd.rs` (Linux) | 변경 없음. 정상. |
| `tui/app.rs` (running_token_count 호출) | 변경 없음. `running_token_count()` API 호환 유지. |
| `tui/event.rs` (is_any_running 호출) | 변경 없음. 호출 코드 동일하나, 내부 동작 변경됨 (항목 3 참조). |
| `cli/install.rs` (is_any_running 호출) | 변경 없음. 호출 코드 동일하나, 내부 동작 변경됨 (항목 3 참조). |
| `cli/update.rs` (is_any_running 호출) | 변경 없음. 호출 코드 동일하나, 내부 동작 변경됨 (항목 3 참조). |
| `Cargo.toml` (의존성) | 변경 없음. serde, serde_json, chrono 모두 이미 존재. |
