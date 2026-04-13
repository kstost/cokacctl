# Issue 02: 이전 로그 오염으로 인한 시작 성공 오판 가능성

- 심각도: **높음**
- 파일: `src/service/taskscheduler.rs` 199행, 236행, 352행
- 발견: Codex 보고서

---

## 현상

`start()`는 시작 전에 error log만 truncate하고 일반 log(`cokacdir.log`)는 유지한다. 이후 `wait_for_running()` → `startup_log_indicates_success()`가 이전 실행의 성공 마커를 읽고, 새 시작이 실패했음에도 성공으로 오판할 수 있다.

## 관련 코드

### start()에서 error log만 truncate (352행)

```rust
// Truncate error log so we only capture fresh errors
let _ = std::fs::File::create(&self.paths.error_log_file);

// 일반 log(cokacdir.log)는 truncate하지 않음
```

### startup_log_indicates_success() (199~208행)

```rust
fn startup_log_indicates_success(&self) -> bool {
    let log_tail = self.read_log_tail(80);  // cokacdir.log의 마지막 80줄
    let success_markers = [
        "Bot connected",
        "Listening for messages",
        "Scheduler started",
        "No pending updates",
    ];
    success_markers.iter().any(|marker| log_tail.contains(marker))
}
```

### wait_for_running()에서 성공 마커 기반 판정 (236~306행)

```rust
fn wait_for_running(&self) -> Result<(), String> {
    // ...
    // Task state가 Running → Ready로 전환된 경우:
    if self.startup_log_indicates_success() {
        // 성공으로 판정 (과거 로그의 마커를 읽을 수 있음)
        return Ok(());
    }
    // ...
    // 타임아웃 후에도:
    if self.startup_log_indicates_success() {
        // 성공으로 판정 (과거 로그의 마커를 읽을 수 있음)
        return Ok(());
    }
    // ...
}
```

## 문제가 되는 이유

`startup_log_indicates_success()`는 `cokacdir.log`의 마지막 80줄에서 성공 마커를 찾는다. 이 로그 파일은 `start()` 시작 시 truncate되지 않으므로, **이전 실행에서 남은 성공 마커가 그대로 존재**한다.

따라서 새 시작이 실패하더라도 (프로세스가 즉시 종료되었더라도) 과거 로그의 성공 마커 때문에 성공으로 판정될 수 있다.

## 발생 가능 시나리오

1. 어제 서비스가 정상 실행되며 `"Bot connected"` 로그를 남김
2. 오늘 설정 오류나 토큰 오류로 인해 새 시작이 즉시 종료됨
3. `error_log`에는 유의미한 에러가 남지 않거나, 로그를 남기기 전에 종료됨
4. `wait_for_running()`이 Task 상태 Running → Ready 전환을 감지
5. `startup_log_indicates_success()`가 어제의 `"Bot connected"` 마커를 발견
6. 시작을 성공으로 판정 — 사용자에게는 성공으로 보이나 실제 프로세스는 이미 죽어있음

## 이전 버전과의 비교

이전 버전(cokacctl_old)은 로그 분석을 하지 않고, 2초 대기 후 `tasklist`에서 실제 cokacdir 프로세스가 존재하는지 직접 확인했다:

```rust
std::thread::sleep(std::time::Duration::from_millis(2000));
if !Self::is_cokacdir_running() {
    // 프로세스가 없으면 실패
    return Err(format!("cokacdir exited immediately: {}", err_output.trim()));
}
```

이 방식은 다른 약점이 있어도 "이번 실행이 실제로 살아있는가"를 직접 확인했다. 현재 방식은 로그 오염 시 이 최소 안전장치가 사라진다.

## 수정 방안

### 방법 A: 일반 로그도 truncate

`start()`에서 error log와 함께 일반 log도 truncate한다:

```rust
// Truncate both logs so we only capture fresh output
let _ = std::fs::File::create(&self.paths.error_log_file);
let _ = std::fs::File::create(&self.paths.log_file);
```

단, 이 방법은 이전 실행 로그를 전부 잃는다는 단점이 있다.

### 방법 B: 시작 시점의 로그 크기를 기록하고, 그 이후 부분만 분석

```rust
// start() 시작 시점의 로그 크기 기록
let log_size_before = std::fs::metadata(&self.paths.log_file)
    .map(|m| m.len()).unwrap_or(0);

// wait_for_running()에서 log_size_before 이후 부분만 읽어 분석
```

### 방법 C: 로그 분석과 함께 프로세스 존재 확인도 병행

`wait_for_running()` 내부에서 로그 분석 결과와 별개로, 최종적으로 실제 프로세스가 살아있는지 `tasklist` 기반 확인을 추가한다.
