# Fixture CSV di PresentMon

Catture vere dello spike M7b del 2026-10-04 (esito in `docs/superpowers/references/m7/spike-findings.md`). Servono da fixture condivise per i test del parser del servizio (.NET) e per le metriche di `oma-core::frames` (Rust).

**Origine:**
- **Macchina:** RTX 4080 con driver NVIDIA 617.14, Windows 11 26300. La frequenza QPC è 10 MHz: `TimeInQPC` si divide per 10⁷ per avere i secondi.
- **Programma:** `PresentMon-2.6.0-x64.exe`, SHA-256 `B2A706BC6AD475749E3B7E3409263AA1E6906D45BDCF993F6DBC0F660188F1AF`.
- **Argomenti:** `--output_file <f> --stop_existing_session --no_console_stats --qpc_time --track_frame_type --write_frame_id --no_track_input`, più `--track_pc_latency` per i file `*-pcl`, con il tracciamento della GPU acceso (colonne `MsGPU*` presenti).

**Riduzione:** 10 s per file, presi dopo i primi 2 s della cattura. Nel file restano l'intestazione completa e le sole righe del PID del gioco, senza altri processi né percorsi. Rispetto al file originale di PresentMon cambia il formato: righe LF e niente BOM. I test del parser devono coprire anche il BOM e CRLF, con casi scritti a mano.

| File | Gioco, API | Caso | Righe | FPS mostrati | `PCLFrameId` (primo → ultimo, FPS renderizzati) | Alternanza / rapporto dei `MsBetweenPresents` | Frame con GPU busy ≥ 0,9 × `MsBetweenAppStart` |
|---|---|---|---|---|---|---|---|
| `nofg.csv` | Control Resonant, DX12 | senza FG, limite GPU | 742 | 74,2 | — | 0,74 / 1,07 | 95% |
| `nofg-pcl.csv` | Control Resonant, DX12 | senza FG, PCL | 970 | 97,0 | 14347 → 15316, 96,9 (921 righe con id) | 0,75 / 1,13 | 92% |
| `cpubound.csv` | Control Resonant, DX12 | senza FG, limite CPU | 1564 | 156,4 | — | 0,56 / 1,25 | 54% |
| `dlssfg.csv` | Control Resonant, DX12 | DLSS FG | 1302 | 130,1 | — | 1,00 / 57,7 | 72% (con FG non significativo) |
| `dlssfg-pcl.csv` | Control Resonant, DX12 | DLSS FG, PCL | 1306 | 130,5 | 43715 → 44367, 65,3 (599 righe con id) | 1,00 / 55,8 | 71% (con FG non significativo) |
| `fsrfg.csv` | Control Resonant, DX12 | FSR FG | 1259 | 125,8 | — | 0,79 / 1,05 | 72% |
| `fsrfg-pcl.csv` | Control Resonant, DX12 | FSR FG, PCL | 1063 | 106,3 | 83648 → 84177, 53,1 (485 righe con id) | 0,73 / 1,06 | 82% |
| `smooth.csv` | God of War 2018, DX11 | NVIDIA Smooth Motion | 1572 | 157,0 | — | 1,00 / 44,2 | 50% (con FG non significativo) |
| `smooth-pcl.csv` | God of War 2018, DX11 | Smooth Motion, PCL | 1572 | 157,0 | 2876 → 3661, 78,5 (786 righe con id) | 1,00 / 45,0 | 50% (con FG non significativo) |

**Come si calcolano i valori della tabella:**
- **FPS mostrati:** `1000 · N / Σ MsBetweenDisplayChange` sulle righe con valore numerico. In questi file tutte le righe sono mostrate.
- **FPS renderizzati:** `(ultimo − primo PCLFrameId) / (tempo fra le due righe)`, considerando solo le righe con `PCLFrameId` diverso da 0.
- **Alternanza e rapporto:** mediane su finestre di 2 s, come nell'esito dello spike.
