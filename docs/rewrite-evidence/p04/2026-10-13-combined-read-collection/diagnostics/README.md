# Initial sandbox refusal

The unchanged existing UnixListener fixture binds failed with EPERM in the
sandbox in Core, container and observations tests. Their source did not change.
The earlier collector source was 54f539a; final runtime acceptance binds ed6bd09.
The approved final-runtime workspace rerun passed outside the sandbox using
local-test permissions; it used only owned fixture sockets, never workstation
Docker. No application guard or test expectation was weakened. This failed
lossless transcript is diagnostic evidence, excluded from accepted case totals.
