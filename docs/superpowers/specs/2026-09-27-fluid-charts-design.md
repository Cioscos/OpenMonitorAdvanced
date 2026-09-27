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
non un dato da usare per KPI, legenda, statistiche o log. Fra due campioni,
un breve tratto piatto mostra l'ultimo valore noto fino al bordo destro e
termina con il punto bianco. Questa proiezione è soltanto grafica: non entra
nello storico, nei KPI, nella legenda, nelle statistiche o nei log. Se l'ultimo
valore della serie è assente, non si mostrano tratto né punto.

## Scelta del renderer dopo la prova release

Il primo percorso, SVG ricalcolato e `uPlot.setScale('x', ...)` a ogni frame,
ha prodotto una resa visiva fluida sulla release WebView2, ma ha superato i
budget. Sulla macchina di sviluppo a 164 Hz, con 8 serie GPU e intervallo di
1 min, CPU complessiva app+WebView2 1,22% e memoria privata 249,9 MB;
Semplificata 1,33% e 193,9 MB. La release precedente, nelle stesse viste,
misurava rispettivamente 0,19%/127,7 MB e 0,23%/115,8 MB. Il protocollo e
le limitazioni delle prove brevi sono in `docs/perf-budget.md`.

La revisione mantiene SVG per i minigrafici e uPlot 1.6.32 per dati, scala Y,
legenda e selezione della vista Avanzata. Aggiunge un canvas trasparente
ritagliato alla zona del grafico e dell'asse temporale. Su quel canvas si
disegnano le curve, il glow, la griglia verticale, le tacche, le etichette X
e il tratto a valore mantenuto di ciascuna serie, con lo stesso colore,
spessore e bagliore della sua linea.
Il canvas si trasla tra due campioni tramite il compositor, senza ricostruire
otto spline e rasterizzare l'intero grafico a ogni frame. uPlot non disegna
più le serie né gli elementi X duplicati; i suoi assi Y e la legenda restano
visibili. Il tracciato SVG di ciascun minigrafico viene ricalcolato ai
campioni e traslato fra essi.

Una prova diagnostica con canvas traslato, che muoveva impropriamente anche
gli assi Y, ha misurato 0,98% CPU e 150,5 MB: indica una riduzione possibile
dei costi, ma non convalida il nuovo renderer né offre margine CPU sicuro.
La revisione va quindi misurata sulla release completa. Se non raggiunge i
criteri, si confronta un renderer WebGL sul medesimo scenario e si aggiorna
di nuovo questo design prima di migrarvi. Non si ripiega silenziosamente su
30 o 15 FPS.

## Tempo, dati e ciclo di rendering

- Un solo coordinatore `requestAnimationFrame` gestisce i grafici visibili.
  La frequenza massima iniziale è 60 FPS; l'interfaccia interna accetta 30 e
  15 FPS per una futura impostazione. I callback ricevono il tempo corrente e
  non moltiplicano i campioni del backend.
- Il grafico Avanzata aggiorna i dati uPlot solo ai nuovi snapshot. La scala
  Y cambia solo quando nuovi campioni modificano il suo intervallo. Tra gli
  snapshot l'orologio monotono produce uno spostamento X comune al canvas
  delle serie e della scala temporale: linee, griglia, tacche, etichette X
  e tratto a valore mantenuto avanzano insieme. Il canvas include il
  campione precedente al bordo sinistro, l'overscan necessario perché il
  tratto mantenuto raggiunga sempre il bordo destro del grafico durante la
  traslazione, e le tacche necessarie prima e dopo la finestra; una clip
  alla zona del grafico impedisce di invadere gli assi Y. Al nuovo snapshot
  il canvas si ridisegna sulla nuova base temporale e la traslazione
  riparte dalla posizione equivalente, senza salto. Tra due campioni un
  frame cambia solo la trasformazione dei livelli già disegnati (il canvas
  statico e il livello fisso del punto): nessuna scrittura di geometria,
  dimensione, attributo o proprietà CSS personalizzata; l'unica eccezione è
  la compensazione dello scarto del cursore, scritta solo mentre il
  puntatore è sopra il grafico (protocollo di misura in
  `docs/perf-budget.md`). Il ritardo recuperato dopo una pausa resta
  limitato: un ritorno dalla tray non produce un'animazione accelerata.
- Il tratto a valore mantenuto di ciascuna serie Avanzata è disegnato nel
  canvas statico composito, insieme alle curve e all'asse X: parte dalla
  posizione X dell'ultimo campione reale e arriva al bordo destro
  dell'overscan del canvas, con lo stesso colore, spessore e bagliore della
  sua linea. Mentre il canvas trasla verso sinistra il tratto raggiunge
  comunque il bordo destro del grafico, ritagliato dall'area del grafico.
  Solo il puntino bianco terminale resta in un piccolo livello fisso sopra
  le linee, con un margine di 3 px perché l'intero punto resti visibile al
  bordo destro e agli estremi della scala Y. Il livello fisso e il punto
  cambiano solo ai campioni, alle ricostruzioni della geometria e durante
  la transizione Y di 180 ms; tra un campione e l'altro cambia solo la
  trasformazione del canvas. Non viene aggiunta una colonna a `uPlot.data`.
  Serie sovrapposte conservano un punto per serie.
- I minigrafici mantengono il loro storico di cinque minuti. `LiveStore`
  affianca ai buffer dei valori un buffer circolare condiviso dei timestamp.
  Geometria e glow SVG si aggiornano ai campioni. L'SVG sta in un wrapper
  HTML traslato tramite trasformazione CSS (livello promosso dal
  compositor); il tracciato e il tratto a valore mantenuto, con il relativo
  overscan, stanno nel contenuto traslato e seguono il medesimo orologio
  monotono. Il puntino terminale resta fisso al bordo destro, fuori dal
  contenuto traslato, e si aggiorna solo ai campioni. Valori e timestamp
  restano limitati alla capacità attuale.
- La scala Y dipende solo dai campioni reali. Se l'intervallo automatico
  cambia, uPlot e il mapping verticale del canvas interpolano insieme per
  180 ms, interrompendo e riancorando la transizione a un nuovo snapshot.
  Le etichette Y seguono la scala interpolata; i valori di KPI e legenda
  restano quelli misurati. Con movimento ridotto il cambio è immediato.
  Temi, lingua, densità pixel e dimensione della finestra invalidano la
  geometria disegnata e la ricostruiscono sulla stessa base temporale.
- I campioni `null`/non finiti interrompono la curva e il tratto mantenuto.
  Il campione immediatamente prima del bordo sinistro resta nel buffer per
  consentire alla spline di attraversare la clip senza che il primo segmento
  visibile scompaia. Un nuovo schema, l'orologio che torna indietro e il
  rientro dopo invisibilità riallineano l'animazione allo storico valido.
  Quando l'ultimo campione di una serie è assente, quella serie non mostra
  tratto mantenuto né punto finale.
- Il ciclo si ferma quando la finestra è nascosta o la vista non usa il
  grafico, e riparte riallineato quando torna visibile. Con
  `prefers-reduced-motion: reduce` non c'è scorrimento continuo: i grafici si
  aggiornano soltanto ai campioni reali; il punto resta al bordo destro se
  l'ultimo valore è valido. Alla distruzione o al cambio vista, animazioni e
  risorse grafiche vengono rilasciate.

## Geometria e stile

Le curve passano per i campioni e non creano picchi oltre il minimo e il
massimo dei segmenti adiacenti. Il canvas riusa la stessa geometria monotona
dei minigrafici o una variante equivalente verificata sugli stessi fixture;
non aggiunge punti nello storico. La clip al bordo sinistro mantiene il
campione precedente per evitare che il primo tratto visibile salti.
Ogni serie conserva il proprio colore per la linea; una seconda passata
stretta e semitrasparente produce il glow. Il punto finale ha centro bianco,
dimensione discreta e un eventuale alone tenue nel colore della serie. Il
punto rappresenta l'ultimo campione effettivamente valido e ancora nella
finestra temporale, ma si colloca sul bordo destro tramite il tratto a valore
mantenuto; non copre un buco dei dati. La resa deve funzionare anche per una
serie piatta, un solo campione e più serie sovrapposte. Griglia e testo non
ricevono glow.
I valori dell'asse X mantengono la formattazione locale e scorrono insieme
alle curve; gli assi Y cambiano solo quando lo richiedono nuovi campioni.

## Verifica e decisione prestazionale

Test automatici coprono la geometria senza overshoot, i buchi, la posizione
del punto e del tratto mantenuto, il campione prima del bordo sinistro, la
traslazione comune di linee e scala X, la condivisione del ciclo, il reset
senza salto, la transizione Y, lo stop/ripresa della visibilità e il movimento
ridotto. Il rendering va osservato nel build release WebView2 sulla macchina
di sviluppo, con i minigrafici della vista Semplificata e con la vista
Avanzata a 8 serie e storico di un'ora. Si misurano
callback `requestAnimationFrame`, tempi di disegno e frame mostrati nella
traccia di prestazioni di WebView2, oltre a CPU e memoria, per almeno
un minuto dopo il riempimento dello storico; si ripete dopo almeno un'ora
visibile per cercare crescita di memoria.

Il risultato atteso è una mediana di almeno 55 FPS su display a 60 Hz e un
95° percentile dei tempi di frame non oltre 20 ms, insieme ai budget
esistenti: CPU dell'app sotto l'1% della macchina a riposo, memoria della
finestra sotto 200 MB e tray sotto 30 MB. Il target FPS vale per la finestra
visibile con movimento normale; i limiti del display, la modalità di movimento
ridotto e una finestra nascosta lo sospendono. Sulla macchina attuale a 164 Hz
si riportano sia frame/s medi sia mediana degli intervalli, senza presentarli
come una prova svolta a 60 Hz. CPU e memoria comprendono app e WebView2.
Se il percorso compositato non soddisfa la prova, si profila, si corregge e
si rimisura. Se resta insufficiente, si confronta un renderer WebGL sullo
stesso scenario e si aggiorna questo design prima di migrarvi. Non si abbassa
silenziosamente il limite a 30 o 15 FPS.
