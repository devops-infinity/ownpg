# Debt

Known shortcuts and deferred work. Every entry names the phase that closes it. The list is reviewed at every phase exit; an entry with no owner phase is a defect.

- RSA SSH keys are not accepted by the in-process SSH transport. The `rsa` crate carries RUSTSEC-2023-0071 (Marvin timing side channel) with no fixed release, so the `rsa` feature of the SSH client is off; ed25519 and ECDSA keys work, and the `system` SSH transport covers an RSA-only setup. Closed when the advisory is resolved upstream (phase 5 review).
