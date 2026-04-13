# 문제 목록

이 폴더는 `cokacctl` (현재)과 `cokacctl_old` (이전) 비교 과정에서 발견된 문제들을 기록한 것이다.
Claude 보고서와 Codex 보고서 양쪽의 발견 사항을 통합하였다.

| # | 파일 | 심각도 | 발견자 | 처리 상태 |
|---|------|--------|--------|----------|
| 01 | [uninstall 프로세스 Kill 누락](01-uninstall-missing-process-kill.md) | 높음 | Claude + Codex 공통 | 수정 반영 |
| 02 | [로그 오염으로 인한 시작 성공 오판](02-log-contamination-false-success.md) | 높음 | Codex | 수정 반영 |
| 03 | [start() 성공 판정과 status() 판정의 논리적 모순](03-start-status-contradiction.md) | 높음 | Codex | 수정 반영 |
| 04 | [stop/remove 에러 전파 연쇄 실패](04-error-propagation-cascading-failure.md) | 중간 | Claude | 수정 반영 |
| 05 | [is_any_running() trait 정의 불일치](05-is-any-running-trait-mismatch.md) | 중간 | Claude + Codex 공통 | 수정 반영 |
