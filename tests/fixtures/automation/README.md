# B1 focused inputs

`due-cases.json` fixes UTC arithmetic and missed-run boundaries independently of database implementation. These are leaf fixtures, not a registered restart/durability qualification.

Frozen alert behavior comes from `pi-health` revision `80593b2`: `alert_evaluator.py` uses two consecutive failures, one opening and one recovery, and never clears an absent resource. `alert_policy.py` disables categories and supports resource silences. Rust cases add explicit unavailable observations and bounded inputs.

Existing schedules/channels still need a checked deployed inventory: backup/update presets and local cron timezone, package reconciliation, protection sync/scrub, ordinary Mattermost webhook settings, and stack *arr event/channel configuration. No channel or schedule is silently retired. Model-driven reporting/supervision and chat transport remain outside B1/P04.
