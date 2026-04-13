# Issue 01: Windows uninstall에서 프로세스 강제 종료 누락

- 심각도: **높음**
- 파일: `src/cli/uninstall.rs` 113~138행
- 발견: Claude, Codex 양쪽 공통

---

## 현상

Windows uninstall 시 `Stop-ScheduledTask` + `schtasks /Delete`만 수행하고, 실제 cokacdir 프로세스를 명시적으로 종료하지 않는다.

## 이전 버전 (cokacctl_old)

```rust
// 1단계: 태스크 삭제
let mut cmd = Command::new("schtasks");
cmd.args(["/Delete", "/TN", "cokacdir", "/F"]);
cmd.creation_flags(0x08000000);
// ...

// 2단계: 모든 cokacdir* 프로세스 강제 종료
let mut ps = Command::new("powershell");
ps.args(["-NoProfile", "-NonInteractive", "-Command",
    "Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' } \
    | ForEach-Object { Write-Output \"Killing PID=$($_.Id) Name=$($_.ProcessName)\"; \
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }"]);
ps.creation_flags(0x08000000);
```

## 현재 버전 (cokacctl)

```rust
// 1단계: 태스크 중지
let mut ps = Command::new("powershell");
ps.args(["-NoProfile", "-NonInteractive", "-Command",
    "Stop-ScheduledTask -TaskName 'cokacdir' -ErrorAction SilentlyContinue"]);
ps.creation_flags(0x08000000);
// ...

// 2단계: 태스크 삭제
let mut cmd = Command::new("schtasks");
cmd.args(["/Delete", "/TN", "cokacdir", "/F"]);
cmd.creation_flags(0x08000000);
// ...

// 프로세스 강제 종료 단계 없음
```

## 문제가 되는 이유

1. **Task Scheduler 외부에서 수동 실행된 cokacdir 프로세스는 종료되지 않음.** 사용자가 커맨드라인에서 직접 cokacdir를 실행한 경우, `Stop-ScheduledTask`는 해당 프로세스에 영향을 주지 않는다.

2. **`Stop-ScheduledTask`가 자식 프로세스를 즉시 종료하지 않을 수 있음.** Task Scheduler의 프로세스 종료는 graceful shutdown이며, cokacdir가 종료 신호를 즉시 처리하지 않으면 프로세스가 잔존한다.

3. **플랫폼 간 동작이 비일관적.** MacOS와 Linux의 uninstall은 서비스 중지 후 `pkill cokacdir`로 모든 프로세스를 명시적으로 종료한다:
   - MacOS (`uninstall.rs:78-86`): `Command::new("pkill").arg("cokacdir")`
   - Linux (`uninstall.rs:102-110`): `Command::new("pkill").arg("cokacdir")`
   - Windows: 명시적 프로세스 종료 **없음**

4. **UX 메시지와 실제 동작의 불일치.** `uninstall.rs:27`의 안내 문구는 여전히 다음과 같이 출력한다:
   ```
   1. Stop service (Task Scheduler delete & kill process)
   ```
   하지만 실제 코드에는 kill process 단계가 없다.

## 발생 가능 시나리오

1. 사용자가 uninstall 실행
2. `Stop-ScheduledTask` → `schtasks /Delete` 완료
3. cokacdir 프로세스가 여전히 살아있음 (수동 실행 또는 종료 지연)
4. 파일 삭제 단계에서 `cokacdir.exe`가 프로세스에 의해 잠겨 삭제 실패
5. 사용자는 "Uninstall complete" 메시지를 보았지만 실제로는 불완전한 상태

## 수정 방안

`schtasks /Delete` 이후에 이전 버전의 프로세스 Kill 로직을 복원한다:

```rust
// schtasks /Delete 이후 추가
dlog!("uninstall", "Killing all cokacdir* processes via PowerShell...");
let mut ps_kill = Command::new("powershell");
ps_kill.args(["-NoProfile", "-NonInteractive", "-Command",
    "Get-Process | Where-Object { $_.ProcessName -like 'cokacdir*' } \
    | ForEach-Object { Write-Output \"Killing PID=$($_.Id) Name=$($_.ProcessName)\"; \
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }"]);
ps_kill.creation_flags(0x08000000);
match ps_kill.output() {
    Ok(out) => {
        let stdout = String::from_utf8_lossy(&out.stdout);
        dlog!("uninstall", "kill cokacdir* exit={}, stdout='{}'",
            out.status, stdout.trim());
    }
    Err(e) => {
        dlog!("uninstall", "kill cokacdir* failed: {}", e);
    }
}
```

UX 문구도 실제 동작과 일치시킨다. 현재 문구 "Task Scheduler delete & kill process"는 kill process가 실제로 수행되어야만 정확하다.
