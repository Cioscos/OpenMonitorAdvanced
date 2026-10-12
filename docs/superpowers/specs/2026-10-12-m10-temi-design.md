# M10: temi dell'app

**Stato:** approvata nel brainstorming del 2026-10-12.

## 1. Obiettivo e ambito

La M10 aggiunge alle impostazioni una sezione «Temi»: quattro temi integrati (tre scuri e uno chiaro) e l'importazione di temi dell'utente da un file `.css`, nello spirito dei temi di Discord ma limitata a un contratto di variabili CSS. La sezione spiega anche come scrivere un tema.

### 1.1 Decisioni del brainstorming

| # | Decisione |
|---|---|
| T1 | Un tema dell'utente può ridefinire **solo le variabili note** del contratto (§2), niente CSS libero: nessuna regola sulle classi dei componenti, nessun `url()`, `@import` o `@font-face`. |
| T2 | Tra i temi integrati c'è **un tema chiaro**: i colori fissi rimasti nei componenti diventano variabili e `--color-scheme` entra nel contratto, così anche i temi degli utenti possono essere chiari. |
| T3 | I temi dell'utente arrivano solo con **«Importa…»**: niente cartella osservata, niente pulsante «Ricarica». |
| T4 | **Convalida in Rust** (`oma-core::theme`, puro), applicazione in TypeScript con `style.setProperty`: il testo del file non diventa mai un `<style>`. |
| T5 | Il tema vale per la finestra principale e per l'editor dell'overlay, non per l'overlay in gioco (Direct2D, ha i suoi colori nei profili). |

### 1.2 Fuori dallo scopo

Overlay in gioco, installer NSIS, un editor dei temi nell'app (selettori di colore), cartella osservata con ricarica al volo, galleria online dei temi.

## 2. Contratto delle variabili

### 2.1 Formato del file

Un file UTF-8 (BOM ammesso), al massimo 64 KiB, con un commento d'intestazione facoltativo e un solo blocco `:root { … }`:

```css
/* @oma-theme 1
   @name Nord Notte
   @author Mario */
:root {
  --color-scheme: dark;
  --bg: #2e3440;
  --accent: #88c0d0;
}
```

- `@oma-theme <n>`: versione del contratto, facoltativa (assente = 1). Un numero maggiore di quello noto si accetta con un avviso: le variabili sconosciute si scartano come sempre.
- `@name`: nome del tema, al massimo 64 caratteri dopo il trim; assente o vuoto = nome del file senza estensione.
- `@author`: facoltativo, al massimo 64 caratteri.
- Nomi e autori sono sempre testo, mai `{@html}`; i caratteri di controllo si tolgono.

### 2.2 Variabili

L'elenco sta in un solo file, `crates/oma-core/src/theme/variables.json` (nome, gruppo, tipo, limiti), letto da Rust con `include_str!` e importato dalla UI per la guida e le anteprime.

| Gruppo | Variabili | Tipo |
|---|---|---|
| Schema | `--color-scheme` | `light` / `dark` |
| Superfici | `--bg`, `--surface`, `--surface-2`, `--border`, `--shadow`, `--backdrop` | colore |
| Testo | `--text`, `--text-muted`, `--on-accent` | colore |
| Accenti e stati | `--accent`, `--accent-2`, `--ok`, `--warn`, `--crit` | colore |
| Grafici | `--series-5`, `--series-6`, `--series-7`, `--series-8`, `--chart-grid` | colore |
| Forma | `--radius` | lunghezza in `px`, 0–24 |
| Carattere | `--font` | elenco di famiglie di font |

`--shadow`, `--backdrop` e `--chart-grid` sono nuove (T2): prendono il posto dei colori fissi dei componenti (§5.4). `--color-scheme` si applica anche come proprietà `color-scheme` di `:root`.

Le variabili di layout (`--topbar-h`, `--strip-h`, `--strip-compact-h`, `--chrome-height`, `--z-*`) restano fuori dal contratto.

### 2.3 Valori

- **Colore:** `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`/`rgba()` e `hsl()`/`hsla()` con argomenti numerici (virgole o spazi, `/` per l'alfa, percentuali dove CSS le ammette) e `transparent`. Non sono ammessi `var()`, `calc()`, `color-mix()`, nomi di colore o altre funzioni.
- **Lunghezza:** un numero con `px`, nei limiti del JSON.
- **Elenco di font:** nomi tra virgolette o identificatori separati da virgole, più le famiglie generiche (`sans-serif`, `serif`, `monospace`, `system-ui`); al massimo 8 nomi da 64 caratteri; niente funzioni.
- Il valore normalizzato (spazi ridotti, esadecimale minuscolo) è quello salvato e applicato. `!important` si scarta con un avviso.

### 2.4 Regole di convalida

- Le variabili mancanti si ereditano dal **tema base** dello stesso schema: `builtin-synthwave` per i temi scuri, `builtin-daylight` per i chiari. Senza `--color-scheme` il tema è scuro.
- Si scartano, con un avviso e il numero di riga: variabili sconosciute, valori non validi, dichiarazioni ripetute (vince l'ultima, avviso sulle precedenti), altri selettori o blocchi, at-rule (`@import`, `@font-face`, `@media`…), proprietà che non sono variabili.
- **Errori bloccanti** (codici `themes.error.<codice>`): `tooLarge` (> 64 KiB), `notUtf8`, `empty` (nessuna variabile valida), `unterminated` (commento, stringa o blocco non chiuso).
- Il parser è un tokenizer scritto a mano (commenti, stringhe, `{`, `}`, `;`, `:`), senza librerie CSS: tutto ciò che non riconosce è un avviso, mai interpretato.

### 2.5 Contrasto

`contrast_warnings` calcola il rapporto di contrasto WCAG 2.x sul tema già unito al tema base (con l'alfa composto su `--bg`):

| Coppia | Minimo |
|---|---|
| `--text` su `--bg`, `--surface`, `--surface-2` | 4,5:1 |
| `--text-muted` su `--bg`, `--surface` | 3:1 |
| `--on-accent` su `--accent` | 4,5:1 |
| `--ok`, `--warn`, `--crit`, `--accent` su `--bg` | 3:1 |

Per i temi integrati sono vincoli verificati dai test; per quelli dell'utente sono solo avvisi («il testo secondario ha un contrasto basso: 2,1:1»).

## 3. Temi integrati

Quattro file nel formato del §2, in `app/src/themes/builtin/<id>.css`, inclusi nel bundle e anche in Rust (`include_str!`) per i test e per `theme_resolved`. Sono anche esempi per chi scrive un tema.

| Id | Nome (i18n) | Schema | Carattere |
|---|---|---|---|
| `builtin-synthwave` | Synthwave | scuro | i colori di oggi (`theme.css`); predefinito e base dei temi scuri |
| `builtin-midnight` | Mezzanotte | scuro | blu e grigi sobri, accenti meno saturi |
| `builtin-daylight` | Giorno | chiaro | fondo quasi bianco, accento viola scuro; base dei temi chiari |
| `builtin-contrast` | Alto contrasto | scuro | nero e bianco, accenti gialli e ciano, bordi marcati |

I temi integrati definiscono tutte le variabili. `theme.css` tiene i valori di Synthwave come ripiego, così l'app resta usabile anche se l'applicazione del tema fallisce; un test controlla che coincidano con `builtin-synthwave.css`.

## 4. Pagina «Temi»

Nuova sezione `app/src/components/settings/ThemesSection.svelte` in `SettingsView`, subito dopo «Generali».

### 4.1 Galleria

Una tessera per tema, prima gli integrati e poi quelli dell'utente in ordine di nome. Ogni tessera ha:

- un'anteprima in miniatura disegnata con le variabili del tema (barra in alto, una tessera finta con una barra, una linea del grafico, i tre colori di stato), con stile locale alla tessera e nessun cambio globale;
- nome, autore, «Chiaro»/«Scuro»;
- il segno del tema attivo.

Un clic applica subito il tema e lo salva in `appearance.theme`; non c'è un «Applica» separato.

### 4.2 Azioni

- **Importa tema…:** finestra di sistema dei file (`.css`), poi `theme_import_preview`. Il riepilogo mostra nome, autore, anteprima, numero di variabili applicate, avvisi (righe scartate, contrasto) e i pulsanti «Aggiungi» e «Annulla». «Aggiungi» chiama `theme_import_confirm` e applica il tema. Un nome già presente (senza distinguere maiuscole) riceve il suffisso « (2)», « (3)»…, come `unique_name` dei profili dell'overlay.
- **Esporta** (solo temi dell'utente): salva la forma normalizzata in un `.css` scelto dall'utente.
- **Elimina** (solo temi dell'utente, con conferma): se il tema era attivo si torna a `builtin-synthwave`.
- **Scarica modello:** salva un `.css` con tutte le variabili e i valori di Synthwave, ognuna con un commento che ne spiega l'uso. Il testo viene da `variables.json` più le chiavi i18n della lingua corrente.

### 4.3 Guida «Crea il tuo tema»

Riquadro espandibile nella pagina (chiavi i18n in `en.json` e `it.json`):

1. esempio minimo di tre righe;
2. tabella delle variabili con gruppo, tipo e significato, generata da `variables.json` (un test controlla che ogni variabile abbia la sua descrizione in entrambe le lingue);
3. regole: cosa si scarta e perché, ereditarietà dal tema base, intestazione con i metadati;
4. sicurezza in due righe: i temi cambiano solo colori, forma e font, non possono caricare file né cambiare il comportamento dell'app;
5. consigli sul contrasto.

I termini tecnici (variabile CSS, contrasto, WCAG, alfa) hanno il tooltip `Term`.

## 5. Flusso dei dati

### 5.1 Rust puro: `crates/oma-core/src/theme/`

- `variables.json` e `variables.rs` (deserializzazione e ricerca).
- `parse.rs`: `parse_theme(bytes: &[u8]) -> Result<ParsedTheme, ThemeError>`; `ParsedTheme { meta: ThemeMeta { name, author, version }, vars: BTreeMap<String, String>, warnings: Vec<ThemeWarning { line, code, detail }> }`.
- `contrast.rs`: luminanza relativa, composizione dell'alfa, `contrast_warnings(&resolved) -> Vec<ThemeWarning>`.
- `resolve.rs`: `resolve(&ParsedTheme, base) -> BTreeMap` (unione con il tema base) e `to_css(&ParsedTheme) -> String` (forma normalizzata con l'intestazione).
- `builtin.rs`: i quattro temi integrati con `include_str!` dai file di `app/src/themes/builtin/`.

### 5.2 Impostazioni

`Settings` riceve una sezione `appearance: Appearance { theme: String }` con predefinito `builtin-synthwave` e `#[serde(default)]`, così le impostazioni vecchie si leggono ancora. L'id è convalidato in lettura: un id che non è integrato e non ha la forma uuid diventa `builtin-synthwave`.

### 5.3 App: `app/src-tauri/src/theme/`

- `store.rs`: cartella `%APPDATA%\OpenMonitorAdvanced\themes\`, un file `<uuid>.css` per tema in forma normalizzata, scritture atomiche con file temporaneo proprio come `overlay/store`, al massimo 100 temi (`themes.error.tooMany`). Ogni lettura ri-convalida il file; i file illeggibili, non validi o con un nome diverso da `<uuid>.css` si saltano.
- `commands.rs`:
  - `themes_list() -> Vec<ThemeSummary { id, name, author, scheme, builtin, preview: BTreeMap }>`, dove `preview` sono le variabili già unite per la miniatura;
  - `theme_import_preview(path) -> ImportPreview { name, author, scheme, vars, warnings, token }`, con il risultato tenuto in memoria per `token` (uno solo alla volta);
  - `theme_import_confirm(token) -> ThemeSummary`;
  - `theme_export(id, path)`, `theme_delete(id)`;
  - `theme_set(id)`: salva `appearance.theme` ed emette `theme-changed` con la mappa unita;
  - `theme_resolved() -> ResolvedTheme { id, vars, fallback: bool }`, dove `fallback` vale `true` se il tema salvato mancava o era corrotto e si è usato Synthwave.
- I comandi stanno in `capabilities/default.json` e in `capabilities/editor.json` solo `theme_resolved`.

### 5.4 UI

- `app/src/lib/theme/apply.ts` (puro più un adattatore DOM): `applyTheme(vars, root)` imposta ogni variabile con `setProperty` e `color-scheme`, e toglie quelle del tema precedente che il nuovo non ha.
- `main.ts` (e l'ingresso dell'editor) chiama `theme_resolved` **prima** di montare l'app e applica le variabili, così non c'è il lampo di Synthwave. Se il comando fallisce restano i valori di `theme.css`. Con `fallback: true` la finestra principale mostra una volta l'avviso `themes.notice.fallback`.
- Le due finestre ascoltano `theme-changed` e riapplicano. Un piccolo store `themeVersion` (contatore) fa rileggere la palette e ridisegnare ai grafici a Canvas: `ChartStrip`, `StripChart`, `HistoryChart`, `Gauge` e la tela dell'editor.
- **Colori fissi:** i colori fissi dei componenti diventano variabili del contratto o `color-mix()` su di esse: ombre e bagliori su `--shadow`, fondi dei dialoghi su `--backdrop`, griglie dei grafici su `--chart-grid`, i grigi testuali su `--text-muted`. Restano fissi i colori che sono dati: i valori predefiniti dei profili dell'overlay nell'editor e i colori di esempio dei test.

## 6. Errori

| Caso | Comportamento |
|---|---|
| File > 64 KiB, non UTF-8, nessuna variabile valida, blocco non chiuso | importazione rifiutata, messaggio `themes.error.<codice>` |
| Righe scartate, contrasto basso, versione futura | avvisi nel riepilogo, importazione permessa |
| Più di 100 temi | `themes.error.tooMany` |
| Tema attivo cancellato o corrotto su disco | Synthwave, avviso `themes.notice.fallback` una volta |
| `appearance.theme` con un id sconosciuto o malformato | Synthwave, senza avviso |
| Errore di scrittura (disco pieno, permessi) | `themes.error.io`, nessun file parziale |

## 7. Test

- **`oma-core::theme`** con fixture in `testdata/themes/`: tema completo, minimo, chiaro; selettori estranei, `url()`, `@import`, `@font-face`, `!important`, `var()`; commenti e stringhe con `}` e `;` dentro; dichiarazioni ripetute; valori fuori limite; BOM; file oltre 64 KiB; non UTF-8; non chiuso. Per ogni fixture: variabili accettate e avvisi con riga e codice.
- **Contrasto:** rapporti noti (nero su bianco 21:1, coppie WCAG di riferimento), alfa composto.
- **Temi integrati:** si convalidano senza avvisi, definiscono tutte le variabili, rispettano i vincoli del §2.5; `builtin-synthwave.css` coincide con `theme.css`.
- **`theme/store`** in una cartella temporanea: importazione, nomi unici, esportazione, eliminazione, tetto, file corrotto o con nome estraneo saltato, scrittura atomica.
- **`Settings`:** lettura senza `appearance`, id malformato.
- **Vitest:** `applyTheme` (imposta, sostituisce e toglie), `ThemesSection` (galleria, riepilogo dell'importazione con avvisi, conferma di eliminazione, tema attivo cancellato), guida generata da `variables.json`, chiavi i18n presenti in entrambe le lingue.

## 8. Verifiche dal vivo

Le azioni le fa l'utente, senza clic sintetici.

- **M1:** cambio di tema con la vista Avanzata aperta e i grafici in movimento; i grafici prendono i colori nuovi senza riavvio.
- **M2:** importazione di un tema di prova con righe scartate e contrasto basso: riepilogo con gli avvisi, tema applicato.
- **M3:** riavvio con un tema non predefinito: nessun lampo di Synthwave.
- **M4:** tema «Giorno» su Semplice, Avanzata, Prestazioni (anche contagiri e classifica), impostazioni ed editor dell'overlay: nessun testo illeggibile né bagliore fuori posto.
- **M5:** cancellazione a mano del file del tema attivo, poi riavvio: Synthwave e avviso.
- **M6:** «Scarica modello», modifica, importazione.
- **M7:** misure con `scripts/measure-footprint.ps1`: con la finestra aperta < 260 MB e < 2,5% di CPU; a riposo nessun costo dei temi.

## 9. Prestazioni

Il tema si applica una volta all'avvio e una a ogni cambio; i grafici ridisegnano solo in quel momento. Nessun thread, watcher o timer nuovo.
