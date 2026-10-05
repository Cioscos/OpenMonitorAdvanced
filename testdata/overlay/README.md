# Fixture della geometria dell'overlay

`geometry-cases.json` è un array di casi per `place_with_extra` (`crates/oma-core/src/overlay/geometry.rs`). La leggono i test di `oma-core` (`geometry_cases_fixture_matches`) e, con la porta TypeScript della geometria, quelli di Vitest (D13): i valori attesi sono scritti a mano, non generati dal codice.

Ogni caso ha questi campi:

- `name`: descrizione del caso.
- `profile`: un profilo JSON valido, accettato da `parse_profile`.
- `area`: l'area di lavoro, `{x, y, w, h}` in pixel fisici (come `PxRect`).
- `dpi`: DPI del monitor.
- `extraCells`: `[larghezza, altezza]` in celle del riquadro del benchmark, oppure `null`.
- `expected`: `{window, profile, extra}`, ciascuno un rettangolo `{x, y, w, h}` in pixel fisici oppure `null` (`profile` è `null` per un profilo senza blocchi, `extra` senza riquadro).
