"""Run the unchanged single-container lifecycle guest (p03_lifecycle_guest via p04_approved_container_guest)
against this candidate's packages. Staged next to a verbatim copy of tests/privileged_vm/*.py."""
import p04_approved_container_guest as container

container.main(package_version='0.4.15+stacklc.1')
