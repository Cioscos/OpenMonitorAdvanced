# M9a: grafico a strisce e layout a tutto schermo

**Stato:** approvata nel brainstorming del 2026-10-09; i mockup sono in `.superpowers/brainstorm/` (esclusa da git).

## 1. Obiettivo e ambito

La M9 lavora sul monitor e si divide in due piani:

- **M9a (questa spec):**
  - il grafico storico della vista Avanzata, cioè `HistoryChart`, rifatto a strisce;
  - il cassetto «Serie»;
  - il layout a finestra grande delle viste Semplice e Avanzata;
  - i dati in più nelle tessere della Semplice.

  Lavora solo sull'interfaccia (Svelte, uPlot, `app/src/`): servizio, protocollo e provider restano invariati.
- **M9b (spec a parte, dopo la M9a):** i sensori nuovi, cioè timing SPD della RAM, pagina della batteria, energia totale e traffico PCIe da NVML, throttling termico Intel dall'MSR e avviso critico dei dischi SATA.

### 1.1 Problemi di oggi (dall'utente)

1. La legenda dice solo quale colore è quale serie: i valori compaiono solo col cursore sul grafico.
2. Un grafico ha al massimo 2 unità (`MAX_UNITS`) e 8 serie (`MAX_SERIES`): carico in %, temperatura e potenza non si vedono insieme.
3. Il pannello «Serie» è un `<details>` che si apre dentro la pagina e spinge il grafico in fondo; con i circa 60 sensori della CPU il grafico finisce sotto l'elenco.
4. A finestra grande la vista Semplice sta in una colonna di 1100 px, tutta in alto, con la quarta tessera da sola nella seconda riga.

### 1.2 Cosa non cambia

- Le prestazioni del grafico:
  - un canvas composito che scorre per frame, con i percorsi ricalcolati solo a ogni dato;
  - la cadenza di 60/30/15 FPS;
  - `prefers-reduced-motion`;
  - la pausa con la finestra nascosta.
- Le finestre 1m, 5m, 30m e 1h.
- La palette Synthwave.
- Le unità scelte nelle impostazioni (°C/°F, byte/bit).
- La selezione salvata per pagina (`persist.ts`).
- Lo storico iniziale dal nucleo.

### 1.3 Fuori da questa spec

- I grafici delle Prestazioni e le sparkline della Semplice: cambiano solo altezza e dati intorno. Il grafico dello stress test tiene `HistoryChart` (decisione dell'utente del 2026-10-09) e cambia solo nelle serie, che sceglie da solo (§5.4).
- Lo zoom e la selezione col trascinamento.
- Le annotazioni.
- L'esportazione del grafico.
- La larghezza della vista Prestazioni, che resta di 1100 px.

## 2. Grafico a strisce

### 2.1 Struttura

- **Una striscia per unità di misura.** La striscia prende l'unità `unit` del sensore nello schema, e tutte le serie con quell'unità condividono l'asse Y di quella striscia. °C e °F, byte/s e bit/s restano una sola striscia, convertita come oggi da `displayScale`.
- **Ordine delle strisce.** Le strisce seguono l'ordine in cui è entrata nella selezione la prima serie di ogni unità: è l'ordine di `chosen`, come in `unitsOf`.
- **Tre colonne per striscia:**
  - **asse Y** a sinistra, con la larghezza fissa di oggi (72 px), uguale in tutte le strisce perché le linee restino allineate;
  - **linee** al centro;
  - **legenda** a destra, larga 220 px.
- **Asse del tempo.** Solo l'ultima striscia mostra le etichette del tempo. Le righe verticali della griglia si vedono in tutte le strisce e cadono sugli stessi istanti.
- **Altezze.** Con una sola striscia il grafico è alto 260 px, come oggi. Con più strisce ognuna è alta 130 px e il grafico cresce in altezza. Non c'è un tetto: la pagina scorre come scorre già sotto la tabella dei sensori.
- **Asse Y.** Ogni striscia ha il proprio, con l'autoscala e le transizioni di oggi (`yRange`, `stepY`, `labelSafeIncrs`) e la griglia orizzontale propria. Il percento parte comunque da 0..100 e i flag da 0..1, come in `scaleOptions`.
- **Cursore condiviso.** Il cursore di una striscia si riflette in tutte, sullo stesso istante (`cursor.sync` di uPlot con una chiave per grafico). La linea verticale appare in ogni striscia.

### 2.2 Limiti tolti

Nel grafico a strisce non ci sono `MAX_SERIES` e `MAX_UNITS`: la selezione scarta solo i sensori che non esistono più. Le selezioni già salvate restano valide così come sono. I limiti, `canAdd` e il testo `advanced.chart.maxSeries` restano solo per `HistoryChart` (§5.4).

### 2.3 Colori

- **Palette per striscia.** La palette (`PALETTE_TOKENS`, 8 colori) riparte da capo in ogni striscia: i colori devono essere distinti solo dentro la stessa striscia, perché ogni striscia ha la sua legenda.
- **Oltre l'ottava serie di una striscia** il colore si ripete con un tratteggio: dalla 9ª alla 16ª serie il tratto è `[6, 4]`, dalla 17ª alla 24ª `[2, 3]`, e poi il ciclo ricomincia.
- **Legenda.** Il campione mostra lo stesso tratteggio della linea.

### 2.4 Legenda con i valori

È nostra, in Svelte, e prende il posto della legenda di uPlot (`legend: { show: false }`). Per ogni striscia ha:

- **Intestazione:** il nome dell'unità (per esempio «Carico · %», «Temperatura · °C»), con le chiavi i18n `advanced.chart.unit.<unit>`.
- **Righe:** una per serie, con:
  - il campione del colore;
  - il nome (`sensorLabel`);
  - il valore **ora**;
  - **min** e **max** della finestra visibile;
  - la **×** che toglie la serie dalla selezione.
- **Valore «ora».** Senza cursore è l'ultimo valore misurato, cioè il valore dell'ultimo snapshot e non quello proiettato dal segmento tenuto. Col cursore su una striscia diventa il valore in quell'istante, in tutte le strisce. Min e max restano quelli della finestra.
- **Min e max** si ricalcolano a ogni dato dalle colonne del buffer della finestra, non a ogni frame. Un buco nei dati (`null`) non conta. Senza valori la riga mostra «—».
- **Clic sul nome** nasconde o mostra la linea, come oggi `setSeries`. La riga resta, attenuata, con i valori ancora aggiornati. Le serie nascoste non contano per l'autoscala.
- **La ×** ha come etichetta accessibile «Togli <nome>» (`advanced.chart.remove`).
- **I numeri** usano cifre tabulari, così i valori non ballano.

### 2.5 Stati

- **Selezione vuota:** il grafico mostra una sola area di 260 px con il testo `advanced.chart.empty`. Il pulsante «Serie» resta disponibile.
- **Tutto sospeso** (dischi in standby e simili): vale il testo `chart.suspended` di oggi, sulla striscia interessata.
- **Sensore sparito dallo schema:** la serie esce dalla selezione al prossimo reseed, come oggi.

## 3. Cassetto «Serie»

- **Apertura.** Si apre col pulsante «Serie · N» (N = serie scelte) nella barra del grafico, a destra delle finestre.
- **Posizione.** Copre **solo la colonna delle linee**: dal bordo destro dell'asse Y al bordo sinistro della legenda, dalla cima della prima striscia al fondo dell'ultima. Assi e legenda restano sempre visibili e leggibili.
- **Grafico più basso del cassetto.** Il grafico si allunga fino all'altezza del cassetto, con un minimo di 360 px a cassetto aperto, senza spingere il resto della pagina oltre quell'altezza. Si riaccorcia alla chiusura.
- **Trasparenza.** Lo sfondo è `--surface` all'88 % di opacità, con il bordo `--accent` e un'ombra, così sotto si intravedono le linee. Non c'è `backdrop-filter`: il grafico sotto si ridisegna a ogni frame. La sfocatura si aggiunge solo se le verifiche dal vivo la chiedono, misurandone il costo.
- **Contenuto:**
  - **ricerca** in cima, che filtra per nome senza distinguere maiuscole e accenti, con il fuoco qui all'apertura;
  - **gruppi per unità**, con lo stesso nome e lo stesso ordine delle intestazioni della legenda. Prima vengono le unità già presenti nel grafico, nell'ordine delle strisce, poi le altre nell'ordine dello schema;
  - **«tutti · nessuno»** su ogni gruppo, che agisce solo sui sensori visibili con il filtro;
  - **righe:** casella, nome e **valore attuale** del sensore (formattato come nella legenda), aggiornato a ogni snapshot;
  - **colonne:** due da 520 px di larghezza del cassetto in su, una sotto.
- **Effetto di una spunta.** Ogni spunta aggiorna subito strisce e legenda, perché la selezione è la stessa.
- **Chiusura:** clic fuori dal cassetto, Esc, o di nuovo «Serie». Alla chiusura il fuoco torna sul pulsante.
- **Accessibilità.** Il cassetto è un `role="dialog"` non modale, con `aria-label` `advanced.chart.series`. Il pulsante ha `aria-expanded`.
- **Animazione.** Il cassetto entra scorrendo da destra in 150 ms. Con `prefers-reduced-motion` compare senza animazione.

## 4. Architettura

- **`StripChart.svelte`** (nuovo, al posto di `HistoryChart.svelte` nelle pagine della vista Avanzata) è il padre. Tiene:
  - selezione, persistenza e finestra;
  - il `ChartBuffer` unico di tutte le serie;
  - lo storico iniziale e il reseed;
  - la pausa con la finestra nascosta;
  - l'iscrizione all'orologio dei frame (`subscribeChartFrame`) e il viewport (`chartViewport`) condivisi;
  - la legenda e il cassetto.

  A ogni frame chiama il disegno di ogni striscia con lo stesso istante, così scorrono insieme.
- **`ChartStrip.svelte`** (nuovo) è una striscia. Ha uno uPlot per l'unità della striscia, con il canvas composito, il punto bianco, il segmento tenuto e le transizioni Y di oggi, estratti da `HistoryChart.svelte`. Riceve dal padre:
  - le colonne del buffer della sua striscia;
  - i convertitori;
  - i colori con il tratteggio;
  - le serie nascoste;
  - la presenza dell'asse del tempo.

  Al padre restituisce l'indice del cursore.
- **Moduli puri in `app/src/lib/advanced/`:**
  - `stripLayout(ids, schema)`, che dà le strisce con unità e serie nell'ordine del §2.1;
  - `stripPalette(count)`, che dà colore e tratteggio per indice dentro la striscia;
  - `windowExtent(column)`, che dà min e max di una colonna senza i `null`;
  - `drawerGroups(sensors, schema, stripUnits, filter)`, che dà gruppi, ordine e filtro del cassetto.

  `scaleLayout` e `canAdd` restano solo per `HistoryChart`.
- **`chartCanvas.ts`** resta com'è. Dal disegno di una striscia escono le etichette del tempo se `timeAxis` è falso.
- **`HistoryChart.svelte`** resta solo per lo stress test, con i suoi limiti di 8 serie e 2 unità (§5.4). `StripChart` lo sostituisce nella vista Avanzata, e i test del canvas composito e delle transizioni Y si copiano per `StripChart`.

## 5. Layout a finestra grande

### 5.1 Larghezza

`main` in `App.svelte` passa da `max-width: 1100px` a una larghezza per vista:

- **Semplice e Avanzata:** 1600 px;
- **Prestazioni:** 1100 px, come oggi.

Sotto i 1100 px non cambia niente.

### 5.2 Vista Semplice

- **Sotto 900 px di contenuto** la griglia resta quella di oggi (`auto-fit, minmax(260px, 1fr)`), con l'altezza naturale.
- **Da 900 px di larghezza in su**, le tessere vanno su due righe che si dividono l'altezza libera sotto il banner, senza scorrimento:
  - 4 tessere: 2 × 2;
  - 5 tessere (due GPU visibili): 3 sopra e 2 sotto, su una griglia di 6 colonne con span 2 e 3;
  - 3 tessere: 2 sopra e 1 sotto, a tutta larghezza;
  - 2 tessere: 1 × 2.

  Questo vale solo se la finestra è alta almeno 720 px. Sotto, le righe hanno l'altezza naturale e la pagina scorre.
- **Sparkline.** Nelle tessere che si allungano, la sparkline occupa lo spazio verticale che resta, con un minimo di 34 px, l'altezza di oggi. `Sparkline.svelte` prende l'altezza dal contenitore invece dei 34 px fissi.
- **Numeri grandi.** Crescono con la finestra: `clamp(28px, 2.2vw, 40px)`.

### 5.3 Dati in più nelle tessere

I dati vengono tutti da sensori già presenti nello schema. Ognuno compare solo se il sensore esiste e ha un valore, senza mai segnaposti vuoti. Le etichette tecniche hanno il tooltip (`Term.svelte`), con un testo semplice.

- **CPU:**
  - in chip: temperatura, potenza del package, clock (oggi già nel sottotitolo) e «Thread più carico». Si scelgono come i KPI della vista Avanzata (`kpisFor('cpu', …)`, `busiestThread`);
  - sotto, «Carico per thread»: una fila di barre, una per thread logico (`load/thread-*`), alte in proporzione allo 0..100 %, con il valore nel tooltip della barra. Il disegno è CSS, senza canvas.
- **GPU:**
  - in chip: temperatura, hotspot (con il tooltip), clock, potenza su limite («35 W / 320 W» se `power/limit` esiste) e ventola in % e RPM;
  - la VRAM diventa una barra come quella della RAM;
  - l'avviso «Limitata dalla potenza» o «Limitata dal calore» compare solo se `flag/throttle-power` o `flag/throttle-thermal` vale 1, con il colore `--warn`.
- **RAM:**
  - in chip, se presenti, «Disponibile» (totale meno usata, come il KPI `available`) e le temperature dei moduli (`temperature/dimm-*`), che richiedono i sensori avanzati.
- **Rete · Dischi:**
  - accanto alla velocità, il nome dell'adattatore e la velocità del collegamento (`throughput/link-speed`);
  - una tabella per disco (tutti i dischi dello schema, nell'ordine della barra laterale) con:
    - lettera o nome;
    - spazio occupato;
    - barra dell'attività (`load/active`);
    - lettura e scrittura;
    - temperatura.

    Un disco con le misure sospese mostra «In standby» (chiave esistente `storage.power.standby`), senza nessuna lettura che lo svegli.

Le funzioni di selezione stanno in `app/src/lib/select.ts`, accanto a quelle di oggi, e sono pure e testate.

### 5.4 Grafico dello stress test

Il grafico della pagina dello stress resta `HistoryChart`, ma le serie le sceglie da solo secondo il test (`RunStatus.component` e `RunStatus.objective`):

| Test | Normale | Stabilità overclock |
|---|---|---|
| **CPU** | temperatura della CPU (°C) + potenza del package (W) | temperatura della CPU (°C) + clock effettivo |
| **RAM** | RAM usata (%) + temperature dei moduli (°C); senza moduli, la temperatura della CPU, dove sta il controller della memoria | uguale |
| **GPU** | temperatura del core e hotspot (°C) + potenza della scheda (W), della GPU in prova | temperatura del core e hotspot (°C) + clock del core |
| **Disco** | temperatura (°C) + lettura e scrittura, del disco in prova | uguale |

- La temperatura della CPU è la prima presente fra `tdie`, `tctl`, `package` e `core-max`, come oggi.
- Un sensore assente si salta; senza nessun sensore, la pagina mostra `performance.run.noChart` come oggi.
- **Niente pannello «Serie»** e nessuna selezione salvata: il grafico mostra sempre le serie del test. Oggi tutti i tipi di stress condividono la selezione salvata `performance`, che passa da un test all'altro.

## 6. Prestazioni

- **Frame.** Per frame il costo è uno spostamento CSS del canvas di ogni striscia più il punto bianco di ogni serie: la crescita è lineare nel numero di strisce, non di serie.
- **Rischio vero: il ricalcolo dei percorsi a ogni dato.** Senza limite di serie, con la finestra di 1 h e molte serie, ogni secondo si ricalcolano molti percorsi spline.
  - Il ricalcolo resta limitato ai punti che lo schermo mostra (`maxPointsFor`).
  - Una striscia fuori dallo schermo (`IntersectionObserver`) non ricalcola né disegna, finché non torna visibile.
- **Misure con la build release e `scripts/measure-footprint.ps1`:**
  - **(a)** la pagina CPU con le serie predefinite;
  - **(b)** la pagina CPU con tutte le serie della pagina selezionate, a 1m e a 1h;
  - **(c)** la vista Semplice a schermo intero.

  I budget sono quelli di `docs/perf-budget.md`:
  - il nucleo a riposo resta sotto l'1 % di CPU;
  - la finestra resta sotto i 200 MB WebView2 compresa.

  Se il caso (b) supera il budget, si abbassa la cadenza dei frame delle strisce con più di 16 serie prima di toccare altro, e lo si annota in `docs/follow-ups.md`.

## 7. Test

- **Vitest, funzioni pure:**
  - `stripLayout`: ordine delle unità, °C e °F nella stessa striscia, sensori ignoti saltati;
  - `stripPalette`: colori 1–8, tratteggi dalla 9ª e dalla 17ª serie, ciclo;
  - `windowExtent`: `null` esclusi, colonna vuota;
  - `drawerGroups`: ordine dei gruppi, ricerca senza maiuscole e accenti, «tutti · nessuno» sui soli sensori filtrati;
  - le nuove funzioni di `select.ts`, con chip assenti quando manca il sensore;
  - la griglia della Semplice per 2, 3, 4 e 5 tessere.
- **Vitest, componenti:**
  - la legenda mostra i valori senza cursore e li cambia col cursore;
  - la × toglie la serie;
  - il cassetto si apre e si chiude con il pulsante, con Esc e col clic fuori, e il fuoco torna al pulsante;
  - una spunta aggiunge una striscia se l'unità è nuova;
  - nessun limite di serie;
  - i test del canvas composito e delle transizioni Y di `HistoryChart.test.ts`, copiati per `StripChart`;
  - la scelta delle serie dello stress per ogni riga della tabella del §5.4, e il grafico dello stress senza «Serie».
- **Verifiche dal vivo** (le fa l'utente, senza input sintetico):
  - la pagina CPU con 3 o più unità;
  - il cassetto con le linee visibili sotto;
  - la Semplice a schermo intero con 4 e, se possibile, 5 tessere;
  - i casi di misura del §6.

## 8. Documentazione

- **`README.md`:** il grafico a strisce e la Semplice a tutto schermo.
- **`CLAUDE.md`:** la struttura di `StripChart`/`ChartStrip` e la M9a nell'elenco delle milestone.
- **`docs/follow-ups.md`:** la sezione «Open: charts (M9a)» con le misure e i punti rimandati.
