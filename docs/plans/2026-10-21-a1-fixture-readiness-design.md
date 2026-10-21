# A1 disposable fixture readiness

The current-source task witness identifies two udev workers from the fixture's
disk setup exiting during the first planning scan. The runtime correctly
refuses that changed task set. Earlier planning fixtures also assumed that a
Type=exec service start guaranteed the first inventory request was ready;
subsequent CLI/RPC reads succeeded after the original HTTP 503.

Add two shared disposable-guest setup helpers. After reader startup, wait for
its fixed Unix socket and a genuine authenticated read-only inventory response.
Bound time, attempts and response bytes; only the exact busy refusal may be
retried after connection. Any other response, deadline or command failure is
fatal. Retain a bounded summary in the fixture result. Subsequent planning and
inventory assertions remain strict.

Before the first A1 planning RPC, run bounded `udevadm settle`, then wait for
the genuine systemd-udevd parent's children to exit naturally. Bind its start
ticks, bound proc reads, child count and elapsed time, and fail if the parent
changes or workers remain. Keep udev running. Record this as fixture setup,
never admission evidence or a reason to ignore a runtime task difference.

Reject unknown-host use before commands. Test permanent/malformed refusal,
deadline, changed daemon identity, persistent workers and bounded proc reads.
Then run the existing disposable installed suite using the exact committed
helpers. No production binary, policy, comparison, effect retry, service ceiling
or host memory floor changes. The previous interrupted private correction is
not acceptance; a completed real mount, replay and restart result is still
required, followed by the remaining A1 fault matrix and wider cutover gates.
