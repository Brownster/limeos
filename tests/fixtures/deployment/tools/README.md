# CopyParty tools fixtures

Frozen reference: `pi-health` commit
`80593b29443ea9b81eb60ed4e5c3c3ac7a236204`, read without executing Python.
`tools_service.py` establishes `/srv/copyparty`, port 3923, stored-key defaults,
and the saved-before-apply behavior. `tools_manager.py` establishes authenticated
status and its request-host URL construction. `pihealth_helper.py` identifies
the fixed payload and service, and the unverified download/arbitrary-arguments
installation path. `tests/test_tools_service.py` and `tests/test_tools.py` supply
default, port override, path validation, status and helper-failure examples.
Source hashes are included in the tools evidence packet.

`config.json` contains 27 synthetic legacy/default and defect cases. Valid stored
objects can omit keys; actual file absence has a separate result. Read failures,
nulls, arrays, coercions, duplicate/unknown keys, malformed/incomplete input and unsupported arguments
never supply a default configuration. Nonempty arguments are explicitly refused,
including the frozen `-v` example; arbitrary legacy options need an explicit
migration decision and typed policy before configuration writes are enabled.

`services.json` contains 20 synthetic captures of the fixed `LoadState` and
`ActiveState` properties. They are supplied text, not captures from an installed
system. Unrecognized combinations, including a masked but active service, remain
unknown; a mask never implies that an existing process stopped. Separate tests
cover nonzero exits, pending/failed/timed-out/incomplete captures, byte limits,
invalid UTF-8 and raw diagnostic privacy.

`wire.json` is the expected proposed cached GET shape. Its payload `present`
means a legacy regular file was observed, without installation authenticity.
Service `active` and saved configuration do not establish applied configuration,
readiness, reachability or file access. No URL comes from Host/forwarded headers.

Domain tests cover independent original timestamps, inclusive age 90, stale and
future withholding, exact config diffs, missing-file save requirements and gates
that remain unresolved even when stored fields match. No fixture authorizes
effects or replaces protected collection, registration or installed acceptance.
