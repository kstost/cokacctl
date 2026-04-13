# Issue 05: is_any_running()이 trait 정의와 불일치

- 심각도: **중간**
- 파일: `src/service/taskscheduler.rs` 471행
- 발견: Claude 보고서 (Codex는 Issue 03 내부에서 부분 언급)

---

## 현상

`is_any_running()`의 trait 정의는 "서비스 매니저와 무관하게 실제 cokacdir 프로세스가 실행 중인지 확인"을 요구하지만, 현재 Windows 구현은 Task Scheduler 상태만 확인한다.

## trait 정의 (service/mod.rs:43-44)

```rust
/// Check if any cokacdir process is running externally (regardless of service manager).
fn is_any_running(&self) -> bool;
```

주석이 명확히 **"regardless of service manager"**를 명시한다.

## 이전 버전 (cokacctl_old) — trait 정의에 부합

```rust
fn is_any_running(&self) -> bool {
    Self::is_cokacdir_running()
    // tasklist에서 모든 cokacdir* 프로세스 검색
    // Task Scheduler 등록 여부와 무관
}
```

`tasklist` 명령으로 시스템의 모든 cokacdir* 프로세스를 검색한다. Task Scheduler로 시작했든, 수동으로 시작했든, 어떤 방식이든 프로세스 존재 여부를 확인한다.

## 현재 버전 (cokacctl) — trait 정의와 불일치

```rust
fn is_any_running(&self) -> bool {
    self.status() == ServiceStatus::Running
    // → query_task_state() → PowerShell Get-ScheduledTask
    // Task Scheduler에 등록되지 않은 프로세스는 감지 불가
}
```

`self.status()`는 `query_task_state()`를 호출하며, 이는 PowerShell로 `Get-ScheduledTask`의 State 속성만 확인한다.

## 다른 플랫폼과의 비교

| 플랫폼 | is_any_running() 구현 | 프로세스 직접 확인 |
|--------|----------------------|------------------|
| MacOS (`launchd.rs:264`) | `pgrep cokacdir` | O |
| Linux (`systemd.rs:311`) | `pgrep cokacdir` | O |
| Windows (현재) | `self.status() == Running` | X |

Windows만 실제 프로세스 확인 대신 서비스 매니저 상태에 의존한다.

## 호출 위치 및 영향

### 1. install.rs:49

```rust
let was_running = mgr.status() == crate::service::ServiceStatus::Running
    || mgr.is_any_running();
```

- **의도:** 설치 전에 cokacdir가 실행 중이었다면, 설치 후 자동으로 재시작
- **문제:** 수동 실행된 cokacdir를 감지 못하면 → 설치 과정에서 바이너리 교체 시 파일 잠김 가능 (Windows에서 실행 중인 .exe는 덮어쓸 수 없음)

### 2. update.rs:63

```rust
let was_running = mgr.status() == crate::service::ServiceStatus::Running
    || mgr.is_any_running();
```

- **의도:** 업데이트 전에 실행 중이었다면, 업데이트 후 자동으로 재시작
- **문제:** install과 동일. 수동 실행 프로세스 미감지 → 바이너리 교체 실패 또는 재시작 누락

### 3. event.rs:384

```rust
if mgr.is_any_running() {
    let stop_result = mgr.stop();
    // ...
}
```

- **의도:** TUI에서 Start 전에 이미 실행 중인 프로세스가 있으면 먼저 중지
- **문제:** 수동 실행 프로세스 미감지 → 중복 실행 가능

## 수정 방안

이전 버전의 구현으로 복원한다. 현재 코드에 `is_cokacdir_running()` 함수가 이미 존재하므로(63~85행) 호출만 변경하면 된다:

```rust
fn is_any_running(&self) -> bool {
    Self::is_cokacdir_running()
}
```

이 변경은 `status()`의 판정과는 독립적으로, trait 정의의 의도("서비스 매니저와 무관한 프로세스 확인")에 부합한다.
