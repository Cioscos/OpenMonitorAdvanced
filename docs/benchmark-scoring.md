# Punteggi dei benchmark e classifica anonima

Questo documento spiega come si calcolano i punteggi di OpenMonitor Advanced, che cosa riceve il server della classifica (un Cloudflare Worker, cartella `scores-worker/`), come si modera e quali dati tratta. Il server è la parte M8d1; l'app che invia i punteggi (pulsante «Condividi», tabella inclusa) arriva con la M8d2. Le decisioni sono nel §7.6 della spec `docs/superpowers/specs/2026-10-06-m8-prestazioni-design.md`.

## Macchine base e formule

Un punteggio non dice «quanto è veloce in assoluto», ma «quanto è più veloce o più lento di una macchina base». Ogni benchmark misura alcune velocità (per esempio quante operazioni al secondo fa la CPU in un carico), le divide per la velocità della macchina base nello stesso carico e ne fa la **media geometrica**. La media geometrica (la radice n-esima del prodotto) evita che un solo carico molto veloce o molto lento decida da solo il risultato. Il file `*-baseline.json` contiene le velocità della macchina base, misurate una volta e compilate nell'app: la scala è «fissa», quindi un punteggio di oggi resta confrontabile con uno di domani finché la versione non cambia.

| Versione | File (`crates/oma-core/src/scores/`) | Formula | Macchina di taratura |
|---|---|---|---|
| `cpu-1` | `cpu-1-baseline.json` | `round(1500 × media geometrica(velocità / riferimento))` sui sei carichi (`ntt`, `hash`, `compress`, `sort`, `fft`, `gemm`); due punteggi, `single` (un thread) e `multi` (tutti i thread, con la velocità multi come somma delle velocità dei thread) | Ryzen 7 7800X3D (B11, 2026-10-08) |
| `gpu-1` | `gpu-1-baseline.json` | `round(1500 × media geometrica(velocità / riferimento))` sui tre carichi di ciascun gruppo: Calcolo (`fma`, `int_hash`, `bandwidth`) e Grafica (`fill`, `texture`, `overdraw`) | RTX 4080 (R1, 2026-10-08) |
| `disk-1` | `disk-1-baseline.json` | `round(1000 × media geometrica(MB/s / riferimento))` sulle 8 misure della prova B1 (lettura e scrittura: sequenziale 1 MiB a coda 8 e a coda 1, casuale 4 KiB a coda 32 e a coda 1) | Fanxiang S880 2TB NVMe (D1, 2026-10-08) |

- **Scala a 1500** per CPU e GPU: la macchina base fa esattamente 1500 punti. **Scala a 1000** per il disco.
- Le basi vengono da misure valide, senza bandiere, della macchina di taratura, con 4 cifre significative (`provisional: false`).
- **Il profilo NVMe B2 del disco** non dà punti: non si può condividere.
- Il codice è in `score.rs` (CPU), `gpu.rs` e `disk.rs`; i piani sono quelli delle milestone M8a2, M8b2 e M8c in `docs/superpowers/plans/`.

## Le categorie della classifica

Il server tiene cinque categorie (`board`), ognuna con la sua tabella:

| Categoria dell'invio | Campo di `scores` | Categoria della classifica |
|---|---|---|
| `cpu` | `single` | `cpu-single` |
| `cpu` | `multi` | `cpu-multi` |
| `gpu` | `compute` | `gpu-compute` |
| `gpu` | `graphics` | `gpu-graphics` |
| `disk` | `points` | `disk` |

Un invio della CPU o della GPU salva due righe (una per punteggio), uno del disco una. `readMBs` e `writeMBs` si accettano ma non si salvano.

**Si confrontano solo punteggi della stessa versione.** Le versioni note sono `cpu-1`, `gpu-1` e `disk-1`; una versione diversa dà `unknown_version`. Se la scala cambia, la versione cambia (`cpu-2`…) e le tabelle ripartono: un punteggio `cpu-1` non si confronta mai con uno `cpu-2`.

## Formato dell'invio

`POST /v1/submit`, corpo JSON di al massimo 16 KiB (16384 byte). I campi sconosciuti si ignorano.

```json
{
  "format": 1,
  "appVersion": "0.6.0",
  "category": "cpu",
  "scoreVersion": "cpu-1",
  "valid": true,
  "overclock": false,
  "scores": { "single": 1500, "multi": 1500 },
  "kernels": [],
  "hardware": { "model": "AMD Ryzen 7 7800X3D 8-Core Processor", "ramGB": 32, "osBuild": "26300" },
  "flags": []
}
```

- `format` è 1; `appVersion` ha la forma `X.Y.Z` (al massimo 16 caratteri); `category` è `cpu`, `gpu` o `disk`; `valid` e `overclock` sono booleani.
- `scores` contiene i campi della tabella sopra come numeri (assenti o `null` non vanno bene). Ogni punteggio deve essere finito, maggiore di 0 e non oltre 100000.
- `kernels` è un elenco di al massimo 32 elementi: il server non ne legge il contenuto.
- `hardware.model` è senza caratteri di controllo e, dopo la normalizzazione, lungo da 1 a 128 caratteri; `ramGB` è un intero da 1 a 4096; `osBuild` ha la forma `26300` o `26300.1234`.
- `flags` ha al massimo 16 stringhe del tipo `^[a-z0-9_]{1,32}$` (per esempio `battery`, `throttling`).
- **Normalizzazione del modello:** si tolgono `(R)`, `(TM)` (senza distinguere maiuscole), `®` e `™`, gli spazi bianchi ripetuti diventano uno solo e si tagliano gli estremi. Il nome mostrato è il risultato; la chiave di confronto è lo stesso testo in minuscolo. Così «AMD Radeon(TM) Graphics» e «AMD Radeon Graphics» sono lo stesso modello.
- **Ordine dei controlli:** schema (`bad_schema`), poi `format` (`bad_format`), versione (`unknown_version`), `valid` (`not_valid`), valori (`bad_value`). Gli esempi validi e non validi stanno nelle fixture comuni `testdata/scores/` (`submissions.json`, `normalize.json`, `aggregate.json`), lette dai test del Worker e, con la M8d2, anche da quelli Rust.
- **Plausibilità:** il valore deve stare fra 0,2 e 5 volte il riferimento (estremi compresi). Il riferimento è la mediana dello stesso modello nell'unione fra la tabella già pubblicata e le righe dell'autore; se il modello non c'è, la mediana della categoria; se la categoria è vuota, l'invio passa. Altrimenti `implausible`.

## Formato della tabella

`GET /v1/reference-scores.json` (e il file incluso nell'app, `crates/oma-core/src/scores/reference-scores.json`, licenza CC0-1.0):

```json
{ "format": 1, "generatedAt": "2026-10-08T03:17:00Z",
  "rows": [ { "category": "cpu-single", "scoreVersion": "cpu-1", "model": "…", "value": 1500, "n": 12, "source": "community" } ] }
```

- `value` è la mediana dei punteggi (un intero), `n` il numero di invii che la compongono, `source` è `community` (dal Worker) o `author` (le misure dell'autore).
- Il file dell'app ha anche `"license": "CC0-1.0"` e solo righe `author`: lo genera `pnpm author-table` dai file dei punteggi dell'autore (solo quelli con `valid: true`, non provvisori e di versione nota; per il disco solo B1 con `points`). Il valore è la mediana arrotondata per categoria, versione e modello. Il Worker pubblica solo le righe `community`; l'app unisce le due tabelle e mostra la fonte di ogni riga.

## Il Worker

| Richiesta | Risposta |
|---|---|
| `GET /v1/reference-scores.json` | `200` con la tabella, `ETag` e `Cache-Control: public, max-age=3600`; `304` senza corpo se `If-None-Match` è uguale all'`ETag` |
| `POST /v1/submit` | `201 {"ok":true}` oppure `{"error":"<codice>"}` |

Codici d'errore (corpo `{"error":"…"}`):

- `400`: `bad_json`, `bad_schema`, `bad_format`, `unknown_version`, `not_valid`, `bad_value`, `implausible`;
- `413`: `body_too_large` (più di 16384 byte, dall'header `Content-Length` o dal corpo letto);
- `429`: `rate_limited`;
- `503`: `daily_cap`;
- `404`: `not_found` (altro percorso); `405`: `method_not_allowed` (metodo sbagliato).

**Ordine dei passi dell'invio:** metodo e percorso, dimensione, rate limit, lettura del JSON, validazione, plausibilità, tetto giornaliero, inserimento. Il rate limit viene prima della lettura del JSON, così lo spam costa poca CPU.

- **Rate limit** (un limite di richieste in un periodo): 5 invii al minuto per indirizzo IP, con il binding `SUBMIT_LIMIT` di Cloudflare. L'IP (`CF-Connecting-IP`, `unknown` se manca) è solo la chiave del limite e non si salva né si restituisce.
- **Tetto giornaliero:** al massimo 2000 invii accettati al giorno (UTC), contati nella tabella `daily`. Oltre il tetto la risposta è `503 daily_cap` e non si inserisce niente. Serve a proteggere il limite di scritture del database da uno script con molti IP.
- **Aggregazione:** un cron giornaliero (03:17 UTC) esegue tre istruzioni SQL (`src/aggregate.ts`): cancella gli invii più vecchi di 24 mesi e i contatori giornalieri più vecchi di 7 giorni, poi riscrive la tabella pubblicata. Per ogni categoria, versione e modello calcola la **mediana** (il valore centrale; con un numero pari di invii, la media dei due centrali arrotondata). Sono esclusi gli invii con `overclock` e i modelli di `hidden_models`, e un modello entra solo con **almeno 3 invii**: una mediana di 3 o più invii non identifica una persona. Le righe grezze restano nel database e non sono mai pubbliche.
- **Database D1** (SQLite gestito da Cloudflare): `entries` (una riga per punteggio, con il giorno, mai l'ora), `hidden_models`, `published` (la tabella già pronta e il suo `ETag`) e `daily`. Lo schema è in `scores-worker/migrations/0001_init.sql`. L'id `submission` (un UUID casuale del server) è interno: serve solo a raggruppare le righe di un invio e non si restituisce mai.

## Moderazione

Gli invii entrano da soli e l'autore interviene dopo, a mano, con `wrangler` dalla cartella `scores-worker/`. Non c'è un pannello web. Sono comandi che toccano il database vero: li esegue solo l'autore, mai un agente o la CI.

```powershell
# ultimi invii di un modello (la chiave è il nome normalizzato, in minuscolo)
wrangler d1 execute oma-scores --remote --command "SELECT submission, day, board, value, overclock, flags FROM entries WHERE model_key = 'amd ryzen 7 7800x3d 8-core processor' ORDER BY id DESC LIMIT 20"

# cancellare un invio intero (tutte le sue righe)
wrangler d1 execute oma-scores --remote --command "DELETE FROM entries WHERE submission = '<uuid>'"

# nascondere un modello dalla tabella pubblica
wrangler d1 execute oma-scores --remote --command "INSERT OR IGNORE INTO hidden_models (model_key) VALUES ('<chiave in minuscolo>')"

# togliere l'esclusione
wrangler d1 execute oma-scores --remote --command "DELETE FROM hidden_models WHERE model_key = '<chiave in minuscolo>'"

# ricalcolare la tabella senza aspettare il cron (stesso SQL del cron)
pnpm recompute
```

Dopo una cancellazione o un'esclusione la tabella pubblica cambia solo al prossimo ricalcolo (il cron, oppure `pnpm recompute`). Se `published.body` si guasta, l'invio continua a funzionare con le sole righe dell'autore.

## Informativa sulla privacy della condivisione

Non è un parere legale: descrive che cosa fa il sistema.

- **Titolare del trattamento:** l'autore del progetto (Cioscos). Contatto: le issue del repository `github.com/Cioscos/OpenMonitorAdvanced`.
- **Quali dati:** quelli dell'invio descritto sopra: punteggi, versione dei punteggi e dell'app, nome normalizzato del modello di CPU, GPU o disco, build di Windows, RAM in GB, avvisi, indicazione di overclock e il giorno dell'invio (mai l'ora). Nessun account, nessun identificativo dell'installazione, nessun nome utente o del computer, nessun numero di serie, e **nessun indirizzo IP salvato** né nel database né nelle risposte. I log del Worker sono spenti (`[observability] enabled = false`).
- **Finalità:** costruire la classifica pubblica, cioè le mediane per modello.
- **Base giuridica:** il consenso, dato inviando i punteggi dopo averne visto l'anteprima nell'app (l'invio non parte mai da solo). Per l'indirizzo IP, che Cloudflare vede e che il rate limit usa contro gli abusi senza salvarlo, il legittimo interesse.
- **Conservazione:** 24 mesi, poi la riga si cancella. Nella tabella pubblica compaiono solo mediane di almeno 3 invii.
- **Responsabile del trattamento:** Cloudflare, Inc., che esegue il Worker e il database; vale il suo accordo sul trattamento dei dati (DPA) incluso nei termini del servizio, e i dati di D1 stanno in UE (giurisdizione `eu`). Certificazione EU-U.S. Data Privacy Framework di Cloudflare: **da verificare** (la pagina `https://www.dataprivacyframework.gov/list` non si è potuta leggere al momento della stesura).
- **Diritti:** poiché un invio è anonimo, l'autore non può riconoscerlo come tuo, quindi non può cancellarlo, correggerlo o fartelo avere su richiesta (art. 11 del GDPR: non è tenuto a identificarti solo per questo). Se un invio è palesemente sbagliato o abusivo, si può segnalare nelle issue indicando il modello e il giorno: l'autore può nascondere il modello o cancellare gli invii che riconosce come abusivi.
- **Scaricare la tabella:** l'app la scarica da sola (al massimo una volta al giorno, si può spegnere). Per il download Cloudflare vede, come per ogni richiesta web, l'indirizzo IP e l'User-Agent; il Worker non li salva.

## Deploy

Lo fa l'utente a mano, dalla cartella `scores-worker/`. Nessun token Cloudflare sta nei segreti di GitHub e la CI esegue solo i test.

1. `pnpm install --frozen-lockfile`.
2. `wrangler login` (apre il browser per l'accesso).
3. `wrangler d1 create oma-scores --jurisdiction eu` crea il database nella giurisdizione UE e stampa il `database_id`.
4. In `wrangler.toml` sostituisci i due segnaposto, segnati da un commento `# OMA:`: il sottodominio di `routes` (`scores.example.invalid`) e il `database_id` (`00000000-0000-0000-0000-000000000000`).
5. `wrangler d1 migrations apply oma-scores --remote` crea le tabelle.
6. `wrangler deploy` (o `pnpm deploy`) pubblica il Worker.
7. L'URL scelto va messo anche nella costante dell'app (M8d2).

**Ripiego per il rate limit:** se il binding `SUBMIT_LIMIT` non funziona nel piano gratuito, si toglie `[[ratelimits]]` (con il controllo nel codice, da concordare) e si crea nel pannello di Cloudflare una regola di rate limit della zona su `POST /v1/submit`, per IP, con lo stesso limite (5 al minuto o il minimo consentito dal piano).
