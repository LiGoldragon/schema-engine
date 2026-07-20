# schema-engine architecture

A real daemon and thin CLI. Typed requests traverse state-bearing Signal → Nexus → SEMA Kameo actors. Nexus performs the real golden-bridge TypeSchema ingestion; SEMA persists only through the central daemon's typed binary socket. The remaining accepted roots use the shared declaration shape after the TypeSchema gate.

## Revisable leans
Legacy ingestion is an explicit edge adapter for the witness. Native `TextualSchema` authoring can replace it without changing the central encoded-form storage contract.
