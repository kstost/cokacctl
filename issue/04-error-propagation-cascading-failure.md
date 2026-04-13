# Issue 04: stop()/remove() 에러 전파로 인한 연쇄 실패 가능성

- 심각도: **중간**
- 파일: `src/service/taskscheduler.rs` 131~170행, 427~456행, 338행
- 발견: Claude 보고서

---

## 현상

이전 버전에서 `stop()`과 `remove()`는 내부 실패와 무관하게 항상 `Ok(())`를 반환하는 best-effort 방식이었다. 현재 버전은 `?` 연산자로 에러를 전파하여, PowerShell 또는 Task Scheduler 단일 실패가 `stop()` → `remove()` → `start()` 전체를 차단하는 cascading failure가 발생할 수 있다.

## 이전 버전 (cokacctl_old) — always-succeed 방식

### stop()

```rust
fn stop(&self) -> Result<(), String> {
    // Stop-ScheduledTask 호출 — 결과 무시, 로그만 기록
    let stop_result = Self::powershell(&format!(
        "Stop-ScheduledTask -TaskName '{}' -ErrorAction SilentlyContinue", TASK_NAME
    ));
    if let Ok(ref out) = stop_result {
        let stderr = decode_output(&out.stderr);
        dlog!("taskscheduler", "Stop-ScheduledTask exit: {}, stderr: '{}'", out.status, stderr.trim());
    }

    // 프로세스 Kill — 결과 무시
    let kill_result = Self::powershell("Get-Process | Where-Object ...");
    match kill_result { /* 로그만 */ }

    self.clear_pid();
    Ok(())  // 항상 성공
}
```

### remove()

```rust
fn remove(&self) -> Result<(), String> {
    let _ = self.stop();  // stop 에러 무시
    let del_result = Self::cmd("schtasks").args(["/Delete", ...]).output();
    if let Ok(ref out) = del_result { /* 로그만 */ }  // 삭제 에러 무시
    std::fs::remove_file(&self.paths.wrapper_script).ok();  // 파일 삭제 에러 무시
    Ok(())  // 항상 성공
}
```

### start() 내 remove 호출

```rust
let remove_result = self.remove();
dlog!("taskscheduler", "remove result: {:?}", remove_result);
// 에러 무시, 계속 진행
```

## 현재 버전 (cokacctl) — strict 방식

### stop() — 실패 전파 가능

```rust
fn stop(&self) -> Result<(), String> {
    self.stop_task_if_present()?;   // PowerShell 실패 시 에러 전파
    self.kill_cokacdir_processes();
    self.clear_legacy_pid_file();
    Ok(())
}
```

### stop_task_if_present() — 2개 실패점

```rust
fn stop_task_if_present(&self) -> Result<(), String> {
    match self.query_task_state()? {          // 실패점 1: PowerShell 실행 실패
        None => { return Ok(()); }
        Some(state) if state != "Running" => { return Ok(()); }
        Some(_) => {}
    }
    let output = Self::powershell(&format!(
        "Stop-ScheduledTask -TaskName '{}'", TASK_NAME
    ))?;                                       // 실패점 2: Stop 명령 실패
    if !output.status.success() {
        return Err(format!("Task stop failed: {}", ...));  // 실패점 3
    }
    Ok(())
}
```

### remove() — 4개 실패점

```rust
fn remove(&self) -> Result<(), String> {
    self.stop_task_if_present()?;              // 실패점 1
    self.kill_cokacdir_processes();
    self.delete_task_if_present()?;            // 실패점 2
    if self.paths.wrapper_script.exists() {
        std::fs::remove_file(&self.paths.wrapper_script)
            .map_err(...)?;                    // 실패점 3
    }
    self.remove_state_file()?;                 // 실패점 4
    self.clear_legacy_pid_file();
    Ok(())
}
```

### start() 내 remove 호출 — 에러 전파

```rust
self.remove()?;  // remove 실패 → start 전체 실패
```

## 연쇄 실패 경로

```
PowerShell 실행 실패 또는 Task Scheduler 서비스 응답 없음
  → query_task_state() 실패
    → stop_task_if_present() Err 반환
      → stop() Err 반환
        → remove() 내부의 stop_task_if_present()? 에서 Err 전파
          → remove() Err 반환
            → start() 내부의 self.remove()? 에서 Err 전파
              → start() 전체 실패
```

## 발생 가능 시나리오

1. **PowerShell 일시적 장애:** Windows Update 직후, PowerShell 모듈 로딩 실패 등으로 PowerShell 실행이 일시적으로 실패
2. **Task Scheduler 서비스 문제:** Task Scheduler 서비스가 비정상 상태이거나 응답 지연
3. **권한 문제:** 다른 사용자 계정으로 등록된 동일 이름 태스크가 존재하여 쿼리는 되나 Stop 권한이 없음
4. **wrapper 파일 잠김:** 바이러스 백신 등이 wrapper .bat 파일을 잠금하여 삭제 실패

이 중 어떤 경우든, 이전 버전에서는 서비스 시작이 가능했지만 현재 버전에서는 불가능하다.

## 영향

- 사용자가 서비스를 시작/재시작하려 할 때, 이전 상태 정리 실패로 시작 자체가 차단됨
- TUI에서 Start 버튼을 누르면 cleanup 에러 메시지가 표시되고 서비스 시작 불가
- `install.rs`와 `update.rs`에서도 `mgr.stop().ok()`로 호출하므로 직접적 영향은 없으나, `event.rs:386`에서 `mgr.stop()` 결과를 로깅하는 부분이 있음

## 수정 방안

### 방법 A: start() 내 remove 호출에서 에러 무시 (최소 변경)

```rust
// start() 내부
let remove_result = self.remove();
dlog!("taskscheduler", "remove result: {:?}", remove_result);
// 에러 무시하고 계속 진행 — 이전 버전과 동일한 동작
```

### 방법 B: remove() 내부에서 개별 단계 에러를 무시

```rust
fn remove(&self) -> Result<(), String> {
    let _ = self.stop_task_if_present();   // 에러 무시
    self.kill_cokacdir_processes();
    let _ = self.delete_task_if_present(); // 에러 무시
    if self.paths.wrapper_script.exists() {
        std::fs::remove_file(&self.paths.wrapper_script).ok();
    }
    let _ = self.remove_state_file();
    self.clear_legacy_pid_file();
    Ok(())  // 항상 성공
}
```

### 방법 C: stop()만 lenient하게 변경

`stop()`은 다른 곳에서도 호출되므로(restart, remove), stop의 실패가 연쇄를 일으키지 않도록 stop만 always-succeed로 변경하고, remove()의 나머지 단계는 strict 유지:

```rust
fn stop(&self) -> Result<(), String> {
    let _ = self.stop_task_if_present();  // 에러 무시
    self.kill_cokacdir_processes();
    self.clear_legacy_pid_file();
    Ok(())
}
```
