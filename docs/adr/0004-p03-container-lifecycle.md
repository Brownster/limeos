# ADR 0004: container lifecycle and bounded logs

Status: accepted for P03 continuation, 2026-10-05. Extends RW-031 and RW-032.

Start, stop and restart share plan-bound human approval, current-grant checks, durable queueing, per-resource locks and protected executor receipts. A closed action enum selects the operation definition and fixed Engine endpoint. Each action has its own independently loaded executor ceiling. Start requires a stopped container; stop and restart require a running container. Plans bind the full ID, image, running state and start timestamp.

Verification requires the same ID and image. Start and restart require a later running incarnation; stop requires the selected incarnation to be stopped. A successful Engine response alone never releases the core or executor resource lock. Interrupted jobs use receipt lookup and independent inspection, with no second effect request.

Restart's absent action field remains its canonical representation. Old restart plan and receipt digests therefore survive migration. Authority schema v4 names the shared plan/result tables for container operations. Receipt schema v3 records the action separately. Older binaries refuse the newer schemas. Existing restart HTTP/task entry points remain compatible; generic container entry points serve all lifecycle actions.

Log reads require a current container-management grant and a separate executor log ceiling for an explicitly managed ID. Reads use fixed non-following stdout/stderr endpoints, at most 200 tail lines, bounded bytes and a deadline. Docker's multiplexed framing is decoded; terminal controls and common credential-bearing lines are removed before returning text. Logs are displayed as escaped text and are never persisted in authority events or interpreted as commands. Viewer inventory access does not grant log access.

All mutation qualification runs in the disposable Debian host. Standard installs default to every effect disabled. Shadow refuses any enabled effect, independently of the supplied plan.
