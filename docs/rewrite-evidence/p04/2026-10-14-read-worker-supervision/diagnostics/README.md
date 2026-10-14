# Cleanup selector regression

These failed pre-acceptance logs bind uncommitted working-source hashes, not
the final accepted runtime. Escaped-child and manager-death tests found live
fixtures adopted by the external runner after premature cleanup acknowledgment.
The pinned rustix 1.1.5 process/wait.rs documentation explains that waitpid(None)
selects only children in the caller’s own process group. The dedicated guardian
requires wait(NOHANG), which selects all its children across groups. That
correction passes the retained escaped-session and orphan-observer regressions.
The fixture observer was corrected to the same selector. Exact marker-line
matching ensures a spawned-child record cannot substitute for a live leaf.

Final cleanup also uses only parent-checked pidfds, never reusable numeric group
signals. Final-source regressions and process inspection found no remaining
owned fixtures. These diagnostic counts are excluded from acceptance totals.
