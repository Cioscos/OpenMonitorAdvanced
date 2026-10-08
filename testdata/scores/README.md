# Fixture condivise della classifica

Regole comuni al Worker (TypeScript) e all'app (Rust): le leggono tutti e due, così non possono divergere (spec §7.6).

| File | Contenuto | Chi lo legge |
|---|---|---|
| `normalize.json` | `[{ input, display, key }]`: la normalizzazione dei nomi dei modelli | Vitest (`scores-worker/test/rules.test.ts`) ora; `cargo test` dalla M8d2 |
| `submissions.json` | `{ valid: [{ name, body }], invalid: [{ name, body, error }] }`: invii accettati e rifiutati, con il codice d'errore atteso | Vitest ora; `cargo test` dalla M8d2 (gli invii validi sono anche l'esempio che l'app deve produrre) |

I file si modificano a mano o con uno script usa e getta, e restano in JSON con rientro di due spazi e fine riga LF.
