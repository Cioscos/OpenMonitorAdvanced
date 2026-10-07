# M8b — esiti dello spike della GPU (spec §5.6)

Data: 2026-10-07. Macchina: Ryzen 7 7800X3D, RTX 4080 (driver NVIDIA attuale) e iGPU AMD Radeon (2 CU RDNA 2), Windows 11 26300. Codice da buttare, cancellato dopo lo spike (shader e programma restano in `spike-gpu/`, accanto a questo file, come riferimento per la M8b): D3D11 con `windows` 0.62, shader HLSL compilati con `D3DCompile`, NVML caricato da System32. Le misure si sono fatte con l'utente al PC, che ha confermato il desktop fluido durante il carico.

## Risposte

### 1. Preemption con invii da 30–50 ms

Un secondo device sulla stessa GPU manda un lavoro minuscolo ogni 50 ms e ne misura l'attesa, mentre il primo tiene la GPU piena di invii di FMA.

| GPU | Invio | Attesa a riposo (p50) | Attesa sotto carico (p50 / p99 / max) |
|---|---|---|---|
| RTX 4080 | 37 ms | 0,16 ms | 11,1 / 14,2 / 14,6 ms |
| RTX 4080 | 97 ms | 0,15 ms | 18,8 / 27,6 / 31,8 ms |
| iGPU AMD | 40 ms | 0,91 ms | 29,6 / 30,1 / 38,2 ms |
| iGPU AMD | 100 ms | 0,92 ms | 49,8 / 50,1 / 50,1 ms |

- Le attese sono sempre più brevi dell'invio: lo scheduler di Windows interrompe il calcolo a metà, a quanti di circa 10–30 ms sulla 4080 e di circa 30–50 ms sulla iGPU.
- Anche un disegno da 6,7 s sulla iGPU (scena non tarata, vedi il punto 3) non ha fatto scattare il TDR: la preemption funziona anche a metà disegno. Il tetto di 30–50 ms per invio resta, perché tiene la latenza bassa e il controllo «invio bloccato» (1 s) significativo.
- **Scelta:** invio tarato a 40 ms sulle GPU dedicate e a 20 ms sulle integrate, dove il quanto è più lungo e il desktop condivide la stessa GPU.

### 2. Esattezza bit per bit fra le due GPU

Su 1.048.576 thread, dopo 1000 passi, con il confronto contro la CPU su un campione di 10.811 thread:

- **FMA FP32 su interi piccoli** (`x ← mad(x, −1, c)`, valori sotto 2²⁴): 0 differenze con la CPU su entrambe le GPU. Il digest di tutte le uscite è identico fra NVIDIA e AMD (`c64364003b9c2325`).
- **Hash su interi** (mul, xor, shift, rotl, add): 0 differenze, con digest identico fra le due GPU (`f4fbf9df3729ed6b`).
- **Scelta:** S1 «esatto» e S2 si verificano contro il riferimento della CPU e fra GPU diverse. Serve la compilazione con `D3DCOMPILE_IEEE_STRICTNESS`, così il compilatore non riassocia le catene di `mad`: l'FMA misurato (45–50 TFLOPS sulla 4080) è vicino al picco teorico, segno che le catene non vengono accorciate.

### 3. Determinismo dell'hash del fotogramma (S6)

Scena di quad ruotati, con texture e fusione alfa, a 1920×1080, con 8 letture di texture in più per pixel.

- Con gli stessi parametri, 10 fotogrammi danno un solo hash, sia sulla 4080 sia sulla iGPU, e anche con un device nuovo.
- Il numero di quad tarato su 40 ms cambia un poco da un avvio all'altro (2973 contro 3166 sulla 4080), e con lui l'hash.
- **Scelta:** S6 fissa i parametri della scena una volta per sessione (taratura all'inizio), poi disegna il fotogramma di riferimento e confronta tutti gli altri con quello. La scena grafica si tara come il calcolo: senza taratura, 20.000 quad costavano 270 ms a fotogramma sulla 4080 e 6,7 s sulla iGPU.

### 4. Compilazione degli shader

- `D3DCompile` a runtime (`d3dcompiler_47.dll`, in System32 su Windows 10 e 11): 1–14 ms per shader.
- `fxc.exe` del Windows SDK 10.0.26100 (`/O3 /Gis`): stesso bytecode, byte per byte della stessa lunghezza (2396 byte per l'FMA), perché usa lo stesso compilatore.
- **Scelta:** bytecode compilato al build con `fxc` in `build.rs` di `oma-load` e incluso con `include_bytes!`. Il Windows SDK c'è già dove si compila Rust con MSVC (serve per il link), anche sui runner di GitHub. Così tutti usano lo stesso bytecode, cosa che conta per confrontare i punteggi in classifica, e `oma-load` non dipende dalla versione di `d3dcompiler_47` del sistema. `build.rs` trova `fxc.exe` nella versione più alta di `Windows Kits\10\bin\<versione>\x64\` (radice da `KitsRoot10` nel registro) e fallisce con un messaggio chiaro se non lo trova.

### 5. Contatore dei replay PCIe (NVML)

`nvmlDeviceGetPcieReplayCounter` risponde sulla 4080 e resta a 0 durante 2,5 minuti di carico pieno. **Scelta:** si legge a inizio e a fine fase, e un aumento diventa un avviso «errori corretti sul collegamento PCIe», non un fallimento. Solo NVIDIA; per AMD e Intel non si mostra.

### 6. Nome del processo e limitazione del driver

Lo stesso eseguibile, con il nome `gpuspike.exe` e poi copiato come `FurMark.exe`, per 60 s ciascuno: 48,2–48,5 TFLOPS, 2760–2775 MHz e 211–213 W in entrambi i casi. Il driver NVIDIA attuale non limita in base al nome, almeno per questo carico. Il nome neutro `oma-load.exe` resta, perché non costa niente. `nvmlDeviceGetCurrentClocksEventReasons` vale `0x400` durante tutto il carico, anche al clock massimo, senza nessuna limitazione visibile: è un bit che NVML non documenta, quindi si ignora e si guardano solo i bit documentati di potenza e temperatura.

### 7. Costo sulla CPU

Con l'attesa a `sleep(1)` e due invii in volo, il processo usa dallo 0 al 3% di un core, con una media di circa l'1%. Con l'attesa attiva il valore è simile, perché il driver blocca da solo il thread che invia quando la coda è piena. **Scelta:** attesa con `sleep(1)`.

## Altri esiti

- **VRAM della iGPU.** `QueryVideoMemoryInfo` (segmento locale) dà 15.647 MB di budget sulla iGPU AMD, che ha 485 MB dedicati: il budget è la RAM di sistema condivisa. La regola del §5.1 («90% meno 400 MB») allocherebbe circa 14 GB di RAM. **Scelta:** sulle GPU integrate S4 si limita al minimo fra il 90% del budget meno 400 MB e il 25% della RAM fisica disponibile al momento, con un tetto di 4 GB.
- **Potenza.** L'FMA puro porta la 4080 a circa 210 W, con il clock al massimo. Non basta per uno stress termico completo, per il quale serve il carico grafico (S5) o il misto S5 + S1 del profilo «Verifica normale». Lo spike non ha misurato la potenza con S5: si vedrà nella M8b.
- **Stabilità della velocità.** La velocità dell'FMA resta fra 48,0 e 48,5 TFLOPS per tutto il minuto: il criterio «stabilità ≥ 97%» (finestra peggiore / migliore) è raggiungibile senza falsi allarmi su una GPU sana.
- **Hash su interi:** 17 TIOPS sulla 4080 e 0,20 sulla iGPU; **FMA:** 45–50 TFLOPS sulla 4080 e 0,28 sulla iGPU. Il rapporto fra le due GPU (circa 170–240 volte) indica che la scala del punteggio GPU, con la 4080 a 1000, darà alle iGPU valori sotto 10: va bene per la classifica, ma il contagiri deve avere una scala adatta (lo fa già la regola 1-2-2,5-5 del §3.3).
