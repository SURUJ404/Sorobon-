# Third-party software and integration notices

This project integrates with and/or references third-party software and public
protocols. Their names remain where needed for accurate technical attribution;
this project does not claim ownership of those projects or imply endorsement.

- **Stellar / Soroban**: network, protocol, SDK and host interfaces. See the
  applicable upstream package licenses and the `UPSTREAM.json` files under
  `crates/soroban-sim-host/` and `crates/soroban-sim-host-p30/`.
- **Wasmtime / Wasmi / Bytecode Alliance Jco**: WebAssembly runtime and component
  tooling. Retain the licenses and notices distributed with the corresponding
  dependencies.
- **OpenZeppelin**: smart-account compatibility references in authorization
  documentation and fixtures. OpenZeppelin is not a project owner or sponsor.
- **Perch**: optional account-policy compatibility adapter and captured test
  fixtures. The adapter references a documented storage layout and error codes;
  retain its attribution and do not present it as an implementation authored by
  this project. Remove the adapter and its fixtures only if you also remove the
  corresponding functionality and tests.
- **Nido**: optional wallet-kit integration in the web demo. It is a third-party
  wallet provider, not this project's brand or identity.
- **Creit Tech Stellar Wallets Kit**: wallet integration framework.

Third-party names, package identifiers, protocol domains and trademarks are
retained only where technically or legally appropriate. Consult each upstream
license before redistribution.
