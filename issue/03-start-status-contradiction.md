# Issue 03: start() 성공 판정과 status() 상태 판정의 논리적 모순

- 심각도: **높음**
- 파일: `src/service/taskscheduler.rs` 236~306행, 458~468행, 471행
- 발견: Codex 보고서

---

## 현상

`wait_for_running()`은 "Task 상태가 Running → Ready로 돌아가더라도 성공일 수 있다"는 전제로 동작하지만, `status()`는 "Ready = Stopped"으로 판정한다. 동일 코드베이스 내에서 같은 Task Scheduler 상태에 대해 상반된 해석이 존재한다.

## 관련 코드

### wait_for_running() — "Ready여도 성공 가능" (245~269행)

```rust
Some(state) if saw_running && state == "Ready" => {
    // Running → Ready 전환 감지
    if self.startup_log_indicates_success() {
        self.append_diagnostic_error(
            "...task return to Ready after Running, but service log shows \
            successful startup markers. Treating as success.",
        );
        return Ok(());  // Ready이지만 성공으로 판정
    }
    if !err_tail.trim().is_empty() {
        if self.benign_error_log_only(&err_tail) {
            return Ok(());  // Ready이지만 성공으로 판정
        }
    }
}
```

타임아웃 후에도 (276~300행):

```rust
if self.startup_log_indicates_success() {
    return Ok(());  // Running을 한번도 못 봤어도 성공 가능
}
if saw_running {
    return Ok(());  // Running을 봤으면 성공
}
```

### status() — "Ready = Stopped" (458~468행)

```rust
fn status(&self) -> ServiceStatus {
    match self.query_task_state() {
        Ok(None) => ServiceStatus::NotInstalled,
        Ok(Some(state)) => match state.as_str() {
            "Running" => ServiceStatus::Running,
            "Ready" | "Queued" | "Disabled" => ServiceStatus::Stopped,  // Ready = Stopped
            other => ServiceStatus::Unknown(other.to_string()),
        },
        Err(e) => ServiceStatus::Unknown(e),
    }
}
```

### is_any_running() — status() 위임 (471행)

```rust
fn is_any_running(&self) -> bool {
    self.status() == ServiceStatus::Running  // status()의 판정에 전적으로 의존
}
```

## 두 전제의 모순

| 함수 | Task 상태 "Ready" 해석 |
|------|----------------------|
| `wait_for_running()` | 성공일 수 있음 (로그 확인 후 판단) |
| `status()` | Stopped (무조건) |

같은 코드베이스 안에서 다음 두 명제가 동시에 존재한다:
- **시작 검증**: "Ready로 돌아가도 서비스는 정상 실행 중일 수 있다"
- **상태 판정**: "Ready이면 정지 상태다"

## 발생 가능 시나리오

1. `start()` 호출 → `wait_for_running()` 실행
2. Task 상태가 Running → Ready로 전환됨
3. `startup_log_indicates_success()`가 성공 마커 발견 → `start()` 성공 반환
4. 직후 TUI 또는 CLI에서 `status()` 호출 → "Ready" → `ServiceStatus::Stopped` 반환
5. 사용자는 방금 시작에 성공했다는 메시지를 보았으나, 상태 표시는 "Stopped"

## 영향 범위

이 모순은 다음 기능들에 파급된다:

| 파일 | 행 | 영향 |
|------|---|------|
| `src/cli/install.rs` | 49 | `was_running` 판정 — 설치 전 실행 중이었는지 판단 실패 가능. 설치 후 자동 재시작 경로 누락 |
| `src/cli/update.rs` | 63 | `was_running` 판정 — 업데이트 전 실행 중이었는지 판단 실패 가능. 업데이트 후 자동 재시작 경로 누락 |
| `src/tui/app.rs` | 78, 129 | 서비스 상태 표시 — 실제 실행 중이어도 Stopped으로 표시 가능 |
| `src/tui/app.rs` | 179 | `running_token_count` — status가 Running이 아니면 token count를 읽지 않음 |
| `src/tui/event.rs` | 384 | `is_any_running()` — 시작 전 기존 프로세스 감지 실패 가능, 중복 실행 위험 |

## 이전 버전과의 비교

이전 버전(cokacctl_old)의 `status()`는 Task Scheduler 상태에만 의존하지 않았다:

```rust
fn status(&self) -> ServiceStatus {
    // 1. schtasks /Query로 태스크 존재 확인
    // 2. is_saved_pid_alive()로 저장된 PID 확인
    // 3. is_cokacdir_running()으로 실제 프로세스 확인 (fallback)
    // 4. 모두 아니면 Stopped
}
```

Task 상태와 무관하게 실제 프로세스 존재 여부를 확인하는 fallback이 있었으므로, Task 상태가 "Ready"여도 프로세스가 살아있으면 Running으로 판정했다.

## 수정 방안

### 방법 A: status()에 프로세스 확인 fallback 추가

Task 상태가 "Ready"이더라도 실제 프로세스가 살아있으면 Running으로 판정한다:

```rust
fn status(&self) -> ServiceStatus {
    match self.query_task_state() {
        Ok(None) => ServiceStatus::NotInstalled,
        Ok(Some(state)) => match state.as_str() {
            "Running" => ServiceStatus::Running,
            "Ready" | "Queued" | "Disabled" => {
                // fallback: 실제 프로세스 확인
                if Self::is_cokacdir_running() {
                    ServiceStatus::Running
                } else {
                    ServiceStatus::Stopped
                }
            }
            other => ServiceStatus::Unknown(other.to_string()),
        },
        Err(e) => ServiceStatus::Unknown(e),
    }
}
```

### 방법 B: wait_for_running()에서 "Ready = 실패"로 통일

`wait_for_running()`의 로그 기반 성공 판정 경로를 제거하고, Task 상태가 "Ready"로 돌아오면 일관되게 실패로 처리한다. 다만 이 경우 `cmd.exe /c` wrapper 방식에서 Task가 빠르게 Ready로 돌아가는 정상 케이스를 놓칠 수 있으므로 주의가 필요하다.

### 방법 C: 이전 버전의 프로세스 확인 방식 병행

`wait_for_running()` 마지막에, 그리고 `status()`에서도 `is_cokacdir_running()`을 보조 확인으로 사용하여 두 함수의 판정 기준을 동일한 실행 모델 위에 올린다.
