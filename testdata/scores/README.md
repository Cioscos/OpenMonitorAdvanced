# Fixture condivise della classifica

Regole comuni al Worker (TypeScript) e all'app (Rust): le leggono tutti e due, così non possono divergere (spec §7.6).

| File | Contenuto | Chi lo legge |
|---|---|---|
| `normalize.json` | `[{ input, display, key }]`: la normalizzazione dei nomi dei modelli | Vitest (`scores-worker/test/rules.test.ts`) e `cargo test` |
| `submissions.json` | `{ valid: [{ name, body }], invalid: [{ name, body, error }] }`: invii accettati e rifiutati, con il codice d'errore atteso | Vitest e `cargo test` (gli invii validi sono anche l'esempio che l'app deve produrre) |
| `format-chars.json` | `{ cf: [[inizio, fine], …] }`: gli intervalli di code point della categoria Unicode `Cf` (caratteri di formato), vietati nel nome grezzo del modello | Vitest (confronto con il runtime dei Worker); `cargo test` (tabella `CF_RANGES`) |
| `aggregate.json` | `{ entries, hidden, expected }`: le `entries` da aggregare, i modelli nascosti e la tabella attesa (ordinata per categoria, versione e chiave del modello) | Vitest (`scores-worker/test/aggregate.test.ts`) |

`cargo test` (`oma-core::scores::board`) legge `normalize.json`, `submissions.json` e `format-chars.json`.

I file si modificano a mano o con uno script usa e getta, e restano in JSON con rientro di due spazi e fine riga LF.
