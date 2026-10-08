These are synthetic proc records, not Docker or installed-container evidence.
The fixture represents a writable root and a read-only mount with an escaped
space. Tests mutate parent, propagation, source and option fields independently
and generate bounded stat records with opaque comm bytes. The private kernel
seam injects process replacement, credentials, root/namespace changes and time.
Real tests separately inspect owned local children without namespace mutation.
