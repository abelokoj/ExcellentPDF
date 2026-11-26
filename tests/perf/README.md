# Performance Instrumentation & Budgets (K.24, plan §13.1)

- `budgets.json`: authoritative Phase 0–9 latency, throughput, and memory ceilings with a `10 %` regression tolerance enforced by `tests/budgets.ps1`.
- Measured in CI (`tests/budgets.ps1`):
  - Core installer size (`tests/size/budgets.json`: `core_installer_mb`, LZX:21 cabinet)
  - Cold start to usable window (`cold_start_to_usable_window_ms`)
  - Open typical PDF to first page (`open_typical_pdf_first_page_ms`)
  - Idle working-set memory with one small document (`idle_memory_one_small_document_mb`)
- Trace export contract (K.24 step 3): performance traces record span names, timestamps, thread IDs, and durations only — zero document filenames, text, or annotation contents.
- Idle energy contract (K.24 step 4): zero background timers or GPU SwapChain presents while the document view is idle.
