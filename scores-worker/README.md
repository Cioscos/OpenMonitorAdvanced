# oma-scores: il server della classifica

Cloudflare Worker (TypeScript) con un database D1 per la classifica anonima dei benchmark di OpenMonitor Advanced. Regole, formati, moderazione, informativa sulla privacy e deploy completo sono in [`docs/benchmark-scoring.md`](../docs/benchmark-scoring.md).

## Comandi

Servono Node 22.18 o più recente e pnpm 10.15.0.

```bash
pnpm install --frozen-lockfile
pnpm test            # Vitest con il runtime dei Worker simulato
pnpm check           # controllo dei tipi (tsc --noEmit)
pnpm author-table    # rigenera crates/oma-core/src/scores/reference-scores.json dai file dei punteggi dell'autore
pnpm recompute       # SOLO L'UTENTE: ricalcola la tabella pubblicata sul database REMOTO
pnpm deploy          # SOLO L'UTENTE: wrangler deploy
```

Gli agenti e la CI non toccano mai l'account Cloudflare: niente `wrangler login`, `deploy`, `d1 … --remote`, `secret`.

## Segnaposto in `wrangler.toml`

Due righe, segnate da un commento `# OMA:`:

- `routes`: il sottodominio (`scores.example.invalid`) da sostituire con il tuo;
- `database_id`: l'id che stampa `wrangler d1 create oma-scores --jurisdiction eu`.

## Deploy in breve

`wrangler login`, `wrangler d1 create oma-scores --jurisdiction eu`, i due segnaposto, `wrangler d1 migrations apply oma-scores --remote`, `wrangler deploy`. Il dettaglio e il ripiego per il rate limit sono nel documento sopra.

## Privacy

Il codice non registra nulla (`console.*` è vietato in `src/`, lo controlla la CI) e `[observability]` è spento. L'indirizzo IP serve solo come chiave del rate limit e non si salva.
