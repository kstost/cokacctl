# cokacctl 변경사항 정밀 검토 보고서

## 1. 검토 목적

현재 프로젝트 `/mnt/hgfs/vmware_ubuntu_shared/cokacctl` 와 이전 버전 `/mnt/hgfs/vmware_ubuntu_shared/cokacctl_old` 를 비교하여, 이번 수정이 다음 관점에서 올바르고 완전하게 이루어졌는지 검토하였다.

- 수정된 내용이 논리적으로 일관적인지
- 수정된 부분이 실제 동작 흐름 전체에 맞게 완전하게 반영되었는지
- 수정된 부분과 관련하여 같이 수정되어야 할 부분이 누락되지 않았는지
- 수정으로 인해 새로운 사이드이펙트가 발생하지 않는지
- 표면상 의도는 맞아 보여도 실제 운용 조건에서 오판정 또는 오동작이 생기지 않는지

이번 검토는 "추가로 있으면 좋은 개선점" 중심이 아니라, 현재 수정이 충분히 완결적이고 정확한지 여부를 따지는 데 초점을 두었다.

## 2. 비교 범위

디렉터리 전체 diff 기준으로 실제 코드 변경이 확인된 파일은 아래 3개였다.

- `src/cli/uninstall.rs`
- `src/core/platform.rs`
- `src/service/taskscheduler.rs`

그 외 차이는 아래와 같았다.

- 현재 프로젝트에만 `Cargo.lock`, `.codex` 존재
- 이전 프로젝트에만 `dist_beta` 존재

즉, 실질적인 동작 변경은 Windows 서비스 관리와 관련된 코드에 집중되어 있었다.

## 3. 검토 방법

다음 순서로 검토했다.

1. 프로젝트 간 파일 단위 diff 확인
2. 변경 파일별 unified diff 비교
3. 변경된 코드의 호출 관계 추적
4. 상태 판정, 시작, 중지, 제거, uninstall, TUI 표시, install/update 연동 흐름 확인
5. 변경 전 구현과 변경 후 구현이 무엇을 포기했고 무엇을 새로 의존하게 되었는지 비교
6. 수정 의도 자체가 코드 내부에서 서로 충돌하는지 검토

실행 기반 검증은 제한이 있었다. 이 환경에는 `cargo` 가 없어 `cargo check` 는 수행되지 못했다.

- 확인 시도 결과: `/bin/bash: line 1: cargo: command not found`

따라서 이 보고서는 정적 분석과 호출 흐름 분석에 기반한 결과이다.

## 4. 이번 수정의 핵심 변화 요약

이번 수정의 본질은 Windows Task Scheduler 관리 방식의 전환이다.

### 4.1 이전 방식

기존 구현은 대체로 아래 모델을 사용했다.

- 작업 스케줄러 등록 여부 확인
- 실제 `cokacdir` 프로세스가 살아 있는지 확인
- PID 파일(`cokacdir.pid`) 저장 후 그 PID가 계속 살아 있는지 추적
- `status()` 에서 PID 파일 또는 실제 프로세스 존재 여부를 중심으로 Running 여부를 판단
- uninstall 시에는 작업 삭제뿐 아니라 `cokacdir*` 프로세스도 강제 종료

### 4.2 현재 방식

변경 후 구현은 아래 방향으로 이동했다.

- PID 파일 기반 추적 제거
- Windows 전용 상태 파일 `~/.cokacdir/windows-service.json` 추가
- `running_token_count()` 는 Windows에서 wrapper script 대신 상태 파일을 읽음
- 시작 검증은 `Task Scheduler state + 로그 내용` 으로 판정
- `status()` 는 실제 프로세스가 아니라 `Get-ScheduledTask` 의 상태 문자열만으로 판정
- uninstall 에서 프로세스 kill 단계 제거

즉, 기존의 "실제 프로세스 존재 확인" 중심 설계에서 "스케줄러 상태와 메타데이터 파일" 중심 설계로 전환되었다.

## 5. 상세 검토 결과

아래 항목들은 단순 권고 수준이 아니라, 현재 수정이 완전하거나 정합적이라고 보기 어려운 실제 결함 가능성으로 판단된다.

---

## 5.1 주요 결함 1: 새 시작이 실패했는데도 이전 로그 때문에 성공으로 오판할 수 있음

### 관련 코드

- `src/service/taskscheduler.rs:173`
- `src/service/taskscheduler.rs:186`
- `src/service/taskscheduler.rs:199`
- `src/service/taskscheduler.rs:236`
- `src/service/taskscheduler.rs:352`

### 변경 내용의 의미

현재 `start()` 는 시작 직전 아래 작업을 수행한다.

- error log 파일만 truncate
- 일반 log 파일은 유지

그 후 `wait_for_running()` 에서 성공 여부를 판정할 때 다음을 사용한다.

- Task Scheduler state
- `cokacdir.log` 의 최근 로그에 `"Bot connected"`, `"Listening for messages"`, `"Scheduler started"`, `"No pending updates"` 같은 성공 마커가 있는지
- `error_log` 에 치명적 에러가 있는지

### 문제의 핵심

일반 로그는 비우지 않으므로, 이전 실행에서 남은 성공 로그가 그대로 존재할 수 있다.

이 상태에서 새로 시작한 프로세스가 즉시 종료되더라도, `startup_log_indicates_success()` 가 과거 로그를 읽고 성공으로 판단할 수 있다.

### 실제로 가능한 오동작 시나리오

1. 어제 서비스가 정상 실행되며 `"Bot connected"` 로그를 남김
2. 오늘 설정 오류나 토큰 오류로 인해 새 시작이 즉시 종료됨
3. `error_log` 에는 유의미한 에러가 남지 않거나, 남기기 전에 종료됨
4. `wait_for_running()` 이 기존 `cokacdir.log` 의 성공 마커를 보고 이번 시작도 성공으로 판단
5. 사용자 입장에서는 시작 성공처럼 보이나 실제 프로세스는 이미 죽어 있음

### 왜 이 문제를 심각하게 봐야 하는가

이 문제는 단순한 메시지 부정확성이 아니라, 시작 성공/실패 검증 자체가 신뢰할 수 없게 되는 문제다.

즉, 새 검증 로직이 더 정교해진 것처럼 보이지만 실제로는 "과거 실행의 흔적"에 오염될 수 있다.

### 이전 버전과의 비교

이전 버전은 거칠지만 다음 방식이었다.

- 시작 후 2초 대기
- 실제 `cokacdir` 프로세스가 존재하는지 검사

이 방식은 다른 약점이 있어도 "이번 실행이 실제로 살아 있는가"라는 점은 직접 확인했다. 반면 현재 방식은 로그 오염 시 그 최소 안전장치가 사라진다.

### 판단

현재 구현은 시작 성공 판정 로직이 완전하지 않다. 새 구조로 바꾸었다면 로그 판정은 최소한 "이번 시작 이후에 기록된 로그"만을 대상으로 해야 하는데, 현재는 그렇지 않다.

---

## 5.2 주요 결함 2: `start()` 성공 기준과 `status()` 판정 기준이 서로 모순됨

### 관련 코드

- `src/service/taskscheduler.rs:241`
- `src/service/taskscheduler.rs:245`
- `src/service/taskscheduler.rs:277`
- `src/service/taskscheduler.rs:458`
- `src/service/taskscheduler.rs:471`
- `src/cli/install.rs:49`
- `src/cli/update.rs:63`
- `src/tui/app.rs:78`
- `src/tui/app.rs:129`

### 현재 코드가 전제하는 것

`wait_for_running()` 은 다음 상황을 성공으로 볼 수 있게 작성되어 있다.

- Task state 가 잠시 `Running` 이었다가 다시 `Ready` 가 됨
- 또는 `Running` 을 충분히 관찰하지 못했더라도 로그상 성공 마커가 있음
- 또는 stderr 에 `[ccserver]` 계열 출력만 있고 치명적 에러가 아님

즉, 이 함수는 "Task Scheduler state 가 계속 `Running` 이 아니어도 실제 서비스는 정상일 수 있다"는 전제를 갖고 있다.

### 그런데 `status()` 는 정반대 전제를 사용함

`status()` 는 아래처럼 동작한다.

- `Running` 이면 `ServiceStatus::Running`
- `Ready`, `Queued`, `Disabled` 이면 `ServiceStatus::Stopped`
- 그 외는 `Unknown`

여기에는 실제 프로세스 확인이 전혀 없다.

즉, 같은 코드베이스 안에서 다음 두 명제가 동시에 존재한다.

- 시작 검증: `Ready` 로 돌아가도 성공일 수 있다
- 상태 판정: `Ready` 는 정지 상태다

이 둘은 논리적으로 충돌한다.

### 이 모순이 실제로 미치는 영향

이 문제는 단순 이론 문제가 아니라, 아래 기능들에 영향을 준다.

- `install` 의 `was_running` 판단
- `update` 의 `was_running` 판단
- TUI 대시보드의 서비스 상태 표시
- `running_token_count()` 사용 여부
- `is_any_running()` 기반의 외부 실행 감지

특히 `is_any_running()` 이 더 이상 "실제 프로세스가 있는가"를 의미하지 않고 그냥 `status() == Running` 으로 축소되었다는 점이 중요하다.

### 가능한 실제 시나리오

1. 서비스 시작 직후 내부 로직상 성공 처리됨
2. Task Scheduler 상태는 곧 `Ready` 로 돌아감
3. `status()` 는 이를 `Stopped` 로 표시
4. TUI 는 서비스가 꺼진 것으로 보임
5. `install` 또는 `update` 시 `was_running` 이 false 로 판단되어 자동 재시작 경로가 누락될 수 있음
6. 사용자는 실제 실행 여부와 무관하게 UI/CLI 상태를 신뢰할 수 없게 됨

### 이전 버전과 비교

이전 버전은 완벽하지는 않았지만 적어도 아래 둘은 했다.

- PID 파일 기반 확인
- PID 파일이 없거나 stale 이면 실제 `tasklist` 기반 fallback 확인

현재 버전은 이 두 안전장치를 모두 제거했다.

### 판단

현재 수정은 서비스의 실행 모델을 완전히 새로 정의하면서도, 시작 성공 판정과 지속 상태 판정을 같은 모델로 맞추지 못했다. 이것은 구현의 정합성 결함이다.

---

## 5.3 주요 결함 3: Windows uninstall 에서 실제 프로세스 종료가 빠져 제거가 불완전해짐

### 관련 코드

- 현재: `src/cli/uninstall.rs:113`
- 현재: `src/cli/uninstall.rs:131`
- 현재: `src/cli/uninstall.rs:27`
- 이전: `cokacctl_old/src/cli/uninstall.rs:125`

### 변경 전후 차이

이전 uninstall 의 Windows 경로는 다음을 수행했다.

- `schtasks /Delete`
- `cokacdir*` 프로세스 강제 종료

현재 uninstall 은 다음만 수행한다.

- `Stop-ScheduledTask -TaskName 'cokacdir'`
- `schtasks /Delete`

프로세스 kill 단계는 제거되었다.

### 왜 이것이 문제인가

현재 `taskscheduler.rs` 자체가 이미 다음 가능성을 인정하고 있다.

- Task Scheduler state 와 실제 서비스 프로세스 상태가 완전히 일치하지 않을 수 있다
- `Running -> Ready` 전이가 있어도 성공으로 볼 수 있다

그렇다면 uninstall 에서 task stop 및 task delete 만으로 정리가 끝난다고 볼 근거도 약하다.

특히 이전 구현이 kill 단계를 갖고 있었던 이유는 다음과 같은 실무적 문제를 막기 위해서다.

- 작업은 제거되었지만 이미 떠 있는 프로세스는 계속 살아 있음
- 바이너리 파일 삭제/교체가 프로세스 점유 때문에 실패
- 사용자는 uninstall 완료 메시지를 봤지만 실제 백그라운드 실행은 남아 있음

### 구현과 UX 메시지의 불일치

현재 안내 문구는 여전히 다음을 출력한다.

- `Stop service (Task Scheduler delete & kill process)`

하지만 실제 구현에는 kill process 단계가 없다.

즉, 문구와 코드도 어긋난다.

### 판단

현재 uninstall 수정은 완전하지 않다. 제거 흐름에서 함께 유지되어야 하는 정리 단계가 빠졌다고 보는 것이 타당하다.

---

## 5.4 Windows 상태 파일 도입 자체는 일관되게 반영되었는가

### 관련 코드

- `src/core/platform.rs:159`
- `src/core/platform.rs:168`
- `src/core/platform.rs:206`
- `src/core/platform.rs:223`
- `src/core/platform.rs:256`
- `src/service/taskscheduler.rs:63`
- `src/service/taskscheduler.rs:73`
- `src/service/taskscheduler.rs:84`
- `src/cli/uninstall.rs:215`

### 확인 결과

Windows 상태 파일 추가 자체는 다음 경로들에 비교적 일관되게 연결되어 있다.

- `ServicePaths` 에 `state_file` 추가
- Windows에서는 `~/.cokacdir/windows-service.json` 사용
- `start()` 에서 상태 파일 생성
- `remove()` 에서 상태 파일 삭제
- `uninstall` 에서 해당 파일 삭제 대상 포함
- `running_token_count()` 는 Windows에서 wrapper 대신 상태 파일 사용

이 부분만 놓고 보면 "새 파일을 도입했는데 remove/uninstall 에서 지우지 않는" 식의 누락은 보이지 않았다.

### 다만 주의할 점

문제는 상태 파일의 존재 자체보다, 이 파일이 실제 실행 상태의 진실한 근거가 아니라는 점이다.

현재 상태 파일에는 아래만 저장된다.

- schema version
- task name
- wrapper script path
- binary path
- token count

즉, 메타데이터일 뿐 "실행 중인지"를 증명하지 않는다.

그런데 실제 UI에서는 `status() == Running` 인 경우만 `running_token_count()` 를 읽도록 되어 있으므로, 이 파일이 직접적으로 상태 오판정의 원인은 아니더라도, 상태 판정이 틀리면 이 값도 함께 의미를 잃는다.

### 판단

상태 파일 도입 그 자체는 비교적 일관되게 반영되었지만, 상태 파일과 상태 판정 모델의 결합은 충분히 완성되지 않았다.

## 6. 변경의 논리성 평가

이번 수정의 큰 방향은 이해 가능하다.

- PID 파일 기반 추적은 취약할 수 있음
- Windows Task Scheduler 환경에서는 상태 파일을 별도로 두고 관리하고 싶을 수 있음
- 시작 실패 원인을 로그로 더 자세히 포착하려는 의도도 타당함

하지만 구현 논리 전체는 아직 완전히 닫히지 않았다.

현재 코드에는 다음과 같은 구조적 불일치가 있다.

1. 시작 성공 판정은 "Task state 만으로는 부족하다"는 전제를 둠
2. 그런데 지속 상태 판정은 거의 전적으로 Task state 만 신뢰함
3. uninstall 은 오히려 실제 프로세스 정리를 약화시킴
4. 로그 기반 판정은 과거 로그 오염을 고려하지 않음

즉, 수정의 방향성은 있을 수 있으나, 설계 전환이 모든 관련 흐름에 끝까지 관철되지는 않았다.

## 7. 부수 영향 분석

이번 변경이 부르는 실제 사이드이펙트는 아래와 같다.

### 7.1 상태 표시 왜곡 가능성

- 서비스는 실제 살아 있으나 `Ready` 때문에 `Stopped` 로 보일 수 있음
- 반대로 새 시작은 실패했으나 과거 로그 때문에 성공처럼 처리될 수 있음

즉, 시작/상태/화면표시가 서로 다른 현실을 보여줄 수 있다.

### 7.2 install/update 재시작 로직 누락 가능성

`was_running` 계산이 `mgr.status() == Running || mgr.is_any_running()` 에 의존하는데, 현재 Windows 에서는 둘이 사실상 같은 의미가 되었다.

이전에는 fallback 으로 실제 프로세스 존재를 잡았지만, 지금은 그렇지 않다. 따라서 서비스가 살아 있어도 업데이트 전에 "실행 중 아님"으로 판단하여 후속 재시작 경로가 누락될 수 있다.

### 7.3 uninstall 후 잔존 프로세스 가능성

- 작업 삭제 후 프로세스 잔존
- 파일 삭제 실패 또는 uninstall 후에도 실제 동작 지속

이는 사용자가 uninstall 완료 메시지를 신뢰할 수 없게 만든다.

## 8. 발견되지 않은 사항

아래 부분은 이번 diff 기준으로는 비교적 문제 없이 연결되어 있었다.

- Windows 상태 파일 경로 추가와 uninstall 삭제 대상 반영
- PID 파일 제거 후 legacy PID cleanup 추가
- start 실패 시 task/state file 정리 시도

다만 이는 "해당 부분만 놓고 보았을 때의 연결성"이며, 전체 동작이 완전하다는 뜻은 아니다.

## 9. 최종 결론

현재 변경은 "일부 개선 의도는 보이지만 수정이 완전하고 올바르게 마무리되었다"고 판단하기 어렵다.

특히 다음 3건은 실제 결함 가능성이 높다.

1. 이전 로그 때문에 새 시작 실패를 성공으로 오판할 수 있음
2. `start()` 성공 판정과 `status()` 상태 판정이 서로 논리적으로 모순됨
3. Windows uninstall 에서 실제 프로세스 kill 단계가 빠져 제거가 불완전함

따라서 이번 수정은 단순 보완 필요 수준을 넘어서, Windows 서비스 관리 모델 전체에서 정합성 재검토가 필요한 상태로 판단된다.

## 10. 요약

한 줄 요약하면 다음과 같다.

이번 수정은 Windows 서비스 상태 추적 모델을 바꾸었지만, 시작 검증, 지속 상태 판정, uninstall 정리 흐름이 동일한 실행 모델을 공유하지 않아 구현이 완결되지 않았고, 실제 오판정 및 잔존 프로세스 같은 사이드이펙트 가능성이 존재한다.
