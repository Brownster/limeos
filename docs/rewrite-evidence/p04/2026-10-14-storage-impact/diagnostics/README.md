# Regression fixture prerequisite refusal

The first local working-source run passed 14 cases and failed one before
impact selection: the added second pool copied the first pool name and
contained only one branch. Frozen pool validation requires unique names
and at least two branches. The corrected fixture uses a unique name and
two disjoint branches on the related member device. It now reaches the
intended test: related membership cannot import unrelated direct consumers
or another pool into selected-device dependents. Selecting both devices
includes those consumers. No frozen configuration rule was weakened.

This failed transcript binds its uncommitted working-source hashes and
design commit, not the accepted runtime. Its counts are excluded from
acceptance totals. Full merged stdout/stderr bytes are retained with hashes.
