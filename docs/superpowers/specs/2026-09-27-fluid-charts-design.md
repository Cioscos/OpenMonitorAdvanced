# Grafici fluidi — intermezzo prima di M5

## Intento e confini

La vista Semplificata e la vista Avanzata devono mostrare grafici che scorrono
continuamente con una cadenza vicina a 60 FPS quando la finestra è visibile. Le
linee devono essere morbide, avere un bagliore leggero nel colore della serie e
terminare con un piccolo punto bianco, come nel riferimento visivo fornito
dall'utente. La leggibilità della palette Synthwave resta prioritaria: il
bagliore non si applica a griglia, assi, etichette o testo. Questo lavoro è un
intermezzo tra M4 e M5; non aggiunge ancora un'impostazione per la frequenza.

I sensori continuano a produrre campioni alla frequenza attuale. Lo scorrimento
fra due campioni cambia soltanto la posizione della finestra temporale, senza
inventare misure. La curvatura è una rappresentazione dei campioni esistenti,
non un dato da usare per KPI, legenda, statistiche o log.

## Scelta del renderer

Si prova prima il renderer esistente: SVG per i minigrafici, uPlot 1.6.32 per
il grafico Avanzata. uPlot mantiene assi, legenda, selezione delle serie e
gestione dei buchi. La sua API separa l'aggiornamento dei dati dalla scala X;
il ridisegno della scala a ogni frame è la prima strada da misurare. Nessun
secondo livello grafico viene aggiunto in anticipo. Cambiare renderer resta
una decisione da prendere solo se la prova sul build release dimostra che
questa strada non raggiunge i criteri di accettazione.

## Tempo, dati e ciclo di rendering

- Un solo coordinatore `requestAnimationFrame` gestisce i grafici visibili.
  La frequenza massima iniziale è 60 FPS; l'interfaccia interna accetta 30 e
  15 FPS per una futura impostazione. I callback ricevono il tempo corrente e
  non moltiplicano i campioni del backend.
- Il grafico Avanzata aggiorna i dati uPlot solo ai nuovi snapshot. Tra gli
  snapshot fa avanzare la finestra X secondo un orologio monotono, limitando
  il ritardo recuperato dopo una pausa, così un ritorno dalla tray non produce
  una lunga animazione accelerata. Assi, curva e punti restano sincronizzati.
- I minigrafici mantengono il loro storico di cinque minuti. `LiveStore`
  affianca ai buffer dei valori un buffer circolare condiviso dei timestamp:
  il rendering lo usa per far scorrere la finestra senza salti al nuovo
  snapshot. Valori e timestamp restano limitati alla capacità attuale.
- I campioni `null`/non finiti interrompono la curva. Un nuovo schema,
  l'orologio che torna indietro e il rientro dopo invisibilità riallineano
  l'animazione allo storico valido. Quando l'ultimo campione di una serie è
  assente, quella serie non mostra il punto finale.
- Il ciclo si ferma quando la finestra è nascosta o la vista non usa il
  grafico, e riparte riallineato quando torna visibile. Con
  `prefers-reduced-motion: reduce` non c'è scorrimento continuo: i grafici si
  aggiornano soltanto ai campioni reali.

## Geometria e stile

Le curve passano per i campioni e non creano picchi oltre il minimo e il
massimo dei segmenti adiacenti. Si usa un tracciato spline di uPlot solo se
rispetta questa condizione; altrimenti si adotta una variante monotona o si
limita la curvatura. SVG e uPlot devono seguire la stessa regola di forma.
Ogni serie conserva il proprio colore per la linea; una seconda passata
stretta e semitrasparente produce il glow. Il punto finale ha centro bianco,
dimensione discreta e un eventuale alone tenue nel colore della serie. Il
punto segue l'ultimo campione effettivamente valido e ancora visibile, senza
coprire un buco dei dati. La resa deve funzionare anche per una serie piatta,
un solo campione e più serie sovrapposte.

## Verifica e decisione prestazionale

Test automatici coprono la geometria senza overshoot, i buchi, la posizione
del punto, la condivisione del ciclo, lo stop/ripresa della visibilità e la
modalità di movimento ridotto. Il rendering va osservato nel build release
WebView2 sulla macchina di sviluppo, con i minigrafici della vista
Semplificata e con la vista Avanzata a 8 serie e storico di un'ora. Si misurano
callback `requestAnimationFrame`, tempi di disegno e frame mostrati nella
traccia di prestazioni di WebView2, oltre a CPU e memoria, per almeno
un minuto dopo il riempimento dello storico; si ripete dopo almeno un'ora
visibile per cercare crescita di memoria.

Il risultato atteso è una mediana di almeno 55 FPS su display a 60 Hz e un
95° percentile dei tempi di frame non oltre 20 ms, insieme ai budget
esistenti: CPU dell'app sotto l'1% della macchina a riposo, memoria della
finestra sotto 200 MB e tray sotto 30 MB. Il target FPS vale per la finestra
visibile con movimento normale; i limiti del display, la modalità di movimento
ridotto e una finestra nascosta lo sospendono. Se uPlot non soddisfa la prova,
si ottimizza il percorso di disegno e si rimisura. Se ancora non basta, si
confronta un renderer Canvas/WebGL alternativo sullo stesso scenario e si
aggiorna questo design prima di migrare. Non si abbassa silenziosamente il
limite a 30 o 15 FPS.
