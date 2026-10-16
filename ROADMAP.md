# OpenVXE — Analisi, bug e roadmap verso la 1.0

Analisi dello stato attuale (`src/main.rs`, 349 righe, commit `e78465b`) con bug, feature mancanti e idee per arrivare a una versione stabile.
Riferimenti nel formato `src/main.rs:riga`.

Legenda priorità: 🔴 alta · 🟠 media · 🟢 bassa

---

## 1. Bug

### ✅ 1.1 DPI oltre 12.750 non funziona (overflow `u8`) (risolto per PAW3395SE)
`src/main.rs:82` — `let val = (dpi / 50) as u8;`

Con DPI max 26.000 il valore è `520`, che troncato a `u8` diventa `8` → il mouse riceve **400 DPI** invece di 26.000. Tutto ciò che supera `255 × 50 = 12.750` viene inviato errato, quindi il range dichiarato nel README (50–26.000) è falso.
Il protocollo quasi certamente prevede un byte alto (probabilmente `p[11]` o bit extra in un byte vicino) che va scoperto sniffando il software ufficiale a DPI > 12.750.
Nel frattempo: limitare il clamp a 12.750 in `modify_setting` (`src/main.rs:258`).

> **Fase 4**: la TUI usa la tabella dei codici PAW3395SE (50–10.000 DPI, passo 50) e abilita la modifica dei DPI solo se `GetMouseCIDMID` risponde 2,31. Restano da fare la codifica CORE26K e i DPI oltre 10.000 (byte `ex`).

> **Verificato sull'hardware** (CID/MID 2,31 = PAW3395SE): chiedendo 1600 DPI la TUI scrive il codice `0x20`, cioè **1400 DPI**, e forza lo stadio 1 a **200 DPI**. La tabella dei codici è in `protocol::paw3395_code`.
>
> **Aggiornamento fase 3** (vedi `docs/PROTOCOL.md`): i byte X/Y non sono `dpi / 50` in generale, la codifica dipende dal sensore (PAW3395SE usa una tabella di codici, CORE26K usa `dpi / 50 - 1` con i bit alti nel terzo byte dello stadio). Il pacchetto DPI scrive inoltre **due stadi** (stadio 1 fisso a codice 4). Serve `GetMouseCIDMID` (`vxe-probe --info`) per scegliere la codifica.

### ✅ 1.2 La UI mostra valori finti, non quelli del mouse (risolto)
`src/main.rs:154-161` — i settaggi partono da default hardcoded (800 DPI, 1000 Hz, ...). Non viene mai letta la configurazione reale.
Conseguenze:
- All'avvio la UI può mostrare valori diversi da quelli attivi.
- La prima pressione di ←/→ invia `default ± step`, non `valore_reale ± step` (es. mouse a 1600 DPI → premo → → il mouse va a 850 DPI).

Serve un comando di lettura config (vedi 2.1).

> **Fase 4**: all'avvio la TUI legge l'EEPROM `0x00–0xBF` con `GetEEPROM` e mostra solo i valori reali.

### 🔴 1.3 Panic lascia il terminale rotto
`src/main.rs:137` (`HidApi::new().unwrap()`) e `src/main.rs:143` (`set_blocking_mode(false).unwrap()`) vengono eseguiti **dopo** `enable_raw_mode()` + `EnterAlternateScreen`. Un panic lascia il terminale in raw mode e nell'alternate screen (serve `reset`).
Fix: inizializzare HID prima di entrare in raw mode, gestire gli errori con `?`/messaggio, e installare un `std::panic::set_hook` che ripristina il terminale.

### 🟠 1.4 Possibile panic con uptime < 60 s
`src/main.rs:152` — `Instant::now() - Duration::from_secs(60)`. Su Linux `Instant` è basato su `CLOCK_MONOTONIC` (parte dal boot): se il programma parte entro 60 s dal boot la sottrazione va in underflow → panic.
Fix: `Instant::now().checked_sub(...)` oppure usare `Option<Instant>` per "mai richiesto".

### ✅ 1.5 Ogni comando viene inviato 2× a ogni interfaccia, errori ignorati (risolto)
`src/main.rs:72-75` — per ogni device si chiama sia `write()` sia `send_feature_report()`, scartando i risultati. Inoltre vengono aperte **tutte** le interfacce HID con quel VID/PID (`src/main.rs:140-147`), incluse quelle di input standard.
Problemi:
- comandi duplicati (innocui solo finché il firmware è idempotente);
- il log scrive `TX: Applied ...` (`src/main.rs:270`) anche se ogni scrittura è fallita.

Fix: filtrare per `usage_page`/`interface_number` del canale vendor, usare un solo metodo di invio, propagare l'errore e mostrarlo nel log.

> **Fase 5**: verificato sul mouse che output report e feature report funzionano entrambi; ora si usa solo l'output report (`write`, come il software ufficiale via WebHID). Errori di invio e lettura propagati con `DeviceError` (`thiserror`).

### ✅ 1.6 Motion Sync sovrascrive il timer di sleep (risolto)
`src/main.rs:112` — il pacchetto Motion Sync contiene anche il tempo di sleep, forzato a `0x05`. Chi aveva impostato un altro sleep dal software ufficiale lo perde silenziosamente.
Fix: rendere lo sleep un settaggio vero (vedi 2.4) e inviare entrambi i valori correnti.

> **Aggiornamento fase 3**: il pacchetto scrive 10 byte da `0xA9`. Oltre a motion sync imposta **debounce a 8 ms** e **sleep a 50 s** (unità da 10 s, non minuti), poi due coppie con complemento non valido su straight-line correction e ripple control. Va riscritto per inviare solo `0xAB`.

> **Fase 4**: la TUI riscrive il blocco `0xA9–0xB2` partendo dai valori letti dal mouse, cambiando solo il campo modificato. Se una coppia del blocco ha il complemento non valido, la scrittura viene rifiutata.

### ✅ 1.7 Batteria: 0% rosso prima della prima lettura (risolto: "Waiting...", stato di carica mostrato)
`battery: 0` (`src/main.rs:151`) è indistinguibile da "batteria scarica". Usare `Option<u8>` e mostrare `--` / "In attesa…" finché non arriva la risposta. La formula `raw * 2` (`src/main.rs:202,204`) andrebbe verificata (step del 2%? scala 0–50?), e manca lo stato di carica.

### ✅ 1.8 Nessuna gestione di disconnessione / hotplug (risolto)
I device vengono aperti una sola volta all'avvio. Se il mouse/dongle viene staccato o va in sleep, l'app continua a scrivere nel vuoto; se il mouse viene collegato dopo l'avvio non verrà mai trovato. Serve un ri-scan periodico (es. ogni 2 s se `devices` è vuoto o se `read` restituisce errore).

> **Fase 5**: un errore di lettura o scrittura chiude il device e azzera lo stato letto (config, batteria, firmware), avvisando se c'erano modifiche non applicate. Ogni 2 s si cerca di nuovo il dongle e alla riconnessione la config viene riletta. Testato staccando e riattaccando il dongle.

### ✅ 1.9 Mouse capture abilitato senza motivo (risolto: rimosso)
`src/main.rs:133` — `EnableMouseCapture` ma nessun evento mouse è gestito. Effetto collaterale: nel terminale non si può più selezionare/copiare il testo (es. i log hex del sniffer). Rimuovere o implementare click/scroll.

### 🟢 1.10 Eventi tastiera non filtrati per `KeyEventKind`
`src/main.rs:227` — con crossterm 0.27 su Windows (e con il protocollo kitty) arrivano sia Press sia Release → ogni tasto modifica il valore due volte. Aggiungere `if key.kind == KeyEventKind::Press`.

### 🟢 1.11 Altri dettagli
- `src/main.rs:322` — il gauge DPI legge `app.items[0]` assumendo che il DPI sia sempre il primo elemento; fragile.
- `src/main.rs:198` — il filtro `sz <= 8 → continue` rende inutili i controlli `sz >= 6` / `sz >= 5` successivi, e scarta eventuali risposte batteria corte.
- `src/main.rs:207` — lo sniffer mostra solo i primi 8 byte: poco utile per il reverse engineering (vedi 3.1).
- `src/main.rs:123` — il checksum `0x36` della richiesta batteria è hardcoded invece di usare `calc_checksum`.
- Loop a 16 ms che ridisegna sempre anche senza cambiamenti: ~60 redraw/s di CPU sprecata.

---

### ✅ 1.12 Le scritture a `0x70` non vengono salvate (risolto: la TUI usa gli indirizzi del vendor)
**Test sull'hardware:** il pacchetto legacy (polling 500 Hz) viene confermato dal mouse, ma l'EEPROM non cambia: il tasto avanti resta intatto e il polling resta 1000 Hz. Quindi polling, LOD, debounce e angle snapping della TUI probabilmente non hanno alcun effetto. Una scrittura `SetEEPROM` corretta a `0x00` funziona invece ed è stata verificata. Testo originale:

Polling rate, LOD, debounce e angle snapping vengono scritti come record di 4 byte all'indirizzo `0x70`. Nella mappa EEPROM del vendor `0x70` è il record del **tasto avanti**, e questi record non hanno un checksum valido. Nessuna fonte oltre OpenVXE conferma lo schema "registri sensore" a `0x70`. Da verificare con `vxe-probe --dump --from 60 --to 78` prima e dopo aver cambiato uno di quei settaggi. Se confermato, usare gli indirizzi del vendor: `0x00` polling, `0x0A` LOD, `0xA9` debounce.

### ✅ 1.13 La batteria usa il comando sbagliato (risolto)
Verificato sull'hardware: `0x17` risponde con lunghezza 10 e dati a zero, e la TUI mostrava sempre 20%. Ora usa `0x04`.

La TUI interroga `0x17`, che per il vendor è `GetFarDistanceMode`. La batteria è `0x04`: percentuale diretta (senza ×2), flag di carica e tensione in mV. È verificata su `373b:1085` da un altro progetto. Da confermare con `vxe-probe --info`.

## 2. Feature mancanti

### ✅ 2.1 Lettura della configurazione dal mouse (fatto)
Il prerequisito per tutto il resto. Sniffare (Wireshark + usbmon, o USBPcap su Windows) cosa fa il software ufficiale all'avvio: quasi sicuramente esiste un comando "read" (probabilmente con lo stesso layout indirizzo/lunghezza dei comandi `0x07`, es. `0x08`) che restituisce la memoria config.

### ✅ 2.2 Stadi DPI multipli (fatto, senza DPI X/Y separati)
Il mouse ha più stadi DPI selezionabili col tasto sotto. Ora si modifica solo uno stadio (indirizzo `0x0c`). Servono: numero di stadi attivi, valore per stadio, stadio corrente, eventualmente DPI X/Y separati (i due byte `p[9]`/`p[10]` sono già X e Y identici).

### 🟠 2.3 Rimappatura tasti e macro
Feature standard dei software di questa famiglia: assegnare a ogni tasto click, DPI±, tasti tastiera, multimedia, disabilitato, macro.

### 🟠 2.4 Altri parametri hardware
- Tempo di sleep / spegnimento (già nel pacchetto Motion Sync).
- Ripple control, se supportato dal sensore.
- Polling rate superiori se si usa un dongle 4K/8K compatibile (verificare).
- Versione firmware di mouse e dongle.
- Modalità di connessione (2.4 GHz / cablato) e PID alternativi: in cablato il PID è probabilmente diverso da `0x1085`.

### 🟠 2.5 Profili e persistenza
- File di config (`~/.config/openvxe/config.toml`, crate `serde` + `toml` + `directories`).
- Salvataggio/caricamento profili nominati (es. "gaming", "desktop").
- Applicazione automatica al collegamento del mouse.

### 🟠 2.6 Modalità CLI non interattiva
Utile per script, keybind del window manager e autostart:
```
openvxe --dpi 1600 --polling 1000
openvxe --battery          # stampa 87
openvxe --profile gaming
```
Crate consigliato: `clap`. Bonus: output JSON per barre di stato (waybar, polybar, i3blocks).

### 🟢 2.7 UX della TUI
- Barra di aiuto con i tasti in basso.
- Inserimento numerico diretto del DPI (Enter → digito 1600).
- Passo DPI variabile (Shift+←/→ = ±500).
- Enter/Space per toggle dei booleani.
- Conferma visiva di successo/errore per ogni settaggio.
- Indicatore "connesso/disconnesso" nell'header.
- Rilevamento automatico del modello nell'header invece della stringa fissa `VXE R1 SE+`.

### 🟢 2.8 Notifiche
Notifica desktop (`notify-rust`) a batteria bassa, eventualmente in un piccolo demone/systemd user service separato dalla TUI.

---

## 3. Qualità del progetto e "chicche"

### 🔴 3.1 Documentare il protocollo (`PROTOCOL.md`)
È la parte di maggior valore del progetto. Struttura emersa dal codice:

| Byte | Significato |
|------|-------------|
| report ID | `0x08` |
| `p[0]` | comando (`0x07` = scrittura, `0x17` = batteria) |
| `p[3]` | indirizzo/registro (`0x0c` DPI, `0x70` settaggi sensore, `0xa9` power) |
| `p[4]` | lunghezza dati |
| `p[5..]` | dati, ogni valore seguito dal suo complemento `0x55 - v` |
| `p[15]` | checksum `0x4D - somma(p[0..15])` |

Sotto-registri a `0x70`: `0x01` angle snap, `0x02` debounce, `0x03` LOD, `0x0b` polling.

Questo schema (checksum `0x55`/`0x4D`, VID `0x373b`) è tipico dei chipset Compx usati da molti mouse cinesi (VXE, ATK, VGN, MCHOSE…): con un layer di astrazione si potrebbero supportare **molti altri modelli** con poco sforzo.

Lo sniffer integrato andrebbe potenziato: pacchetto completo, timestamp, TX oltre a RX, salvataggio su file, pausa/scroll.

### 🔴 3.2 Rifattorizzare in moduli e aggiungere test
Struttura proposta:
```
src/
  main.rs        // argomenti, setup terminale
  protocol.rs    // encode/decode pacchetti, checksum (puro, testabile)
  device.rs      // discovery HID, hotplug, I/O
  app.rs         // stato
  ui.rs          // rendering ratatui
```
`protocol.rs` senza dipendenze da HID permette test unitari su ogni pacchetto (confronto con dump reali del software ufficiale). Un trait `Transport` permette un mock per i test end-to-end.

### 🟠 3.3 Packaging e installazione (in parte risolto)
- Il README è troncato: il blocco ` ```bash ` non è chiuso, c'è un `# OpenVXE` duplicato, mancano le sezioni udev, build e utilizzo.
- Fornire il file delle regole udev nel repo, es. `udev/70-openvxe.rules`:
  ```
  KERNEL=="hidraw*", ATTRS{idVendor}=="373b", ATTRS{idProduct}=="1085", TAG+="uaccess"
  ```
- Pacchetto AUR (`openvxe` / `openvxe-git`), magari anche `.deb`/Flatpak.
- ✅ `Cargo.toml`: il nome è `vxe_driver` (binario `vxe_driver`), meglio `openvxe`; aggiungere `description`, `license`, `repository`, `readme`. (Fase 5: pacchetto, libreria e binario rinominati `openvxe`, licenza `GPL-3.0-only`, `rust-version` 1.88 come ratatui 0.30.)
- Aggiungere un file `LICENSE` (il README dice "open-source" ma non c'è licenza → legalmente nessuno può contribuire/redistribuire).

### ✅ 3.4 CI e pulizia codice (risolto)
- GitHub Actions: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, build release.
- Clippy oggi segnala 3 warning: `upper_case_acronyms` (`DPI`, `LOD` → `Dpi`, `Lod`) e `inherent_to_string` (implementare `Display` per `Setting`).
- I commenti nel codice sono in russo (`src/main.rs:200,206,278,...`): tradurli in inglese per i contributori.
- Aggiornare le dipendenze (`ratatui 0.26` → ultima, che porta crossterm più recente).
- ✅ Usare `anyhow`/`thiserror` per gli errori (fase 5: `thiserror` nella libreria, `anyhow` nei binari).

### 🟢 3.5 Multipiattaforma
`hidapi` funziona anche su Windows e macOS: con le fix 1.10 e il filtro per interfaccia, il supporto è quasi gratis. Su Windows serve l'interfaccia vendor corretta perché le interfacce mouse sono bloccate dal sistema.

### 🟢 3.6 Idee extra
- Tema colori configurabile.
- Grafico storico della batteria (sparkline ratatui) con stima dell'autonomia residua.
- Test di polling rate reale (misura degli eventi di movimento).
- Export/import della config in formato compatibile con altri utenti (condividere profili).

---

## 4. Ordine di lavoro consigliato

1. ✅ **Base solida** (completata): fix 1.3, 1.4, 1.10, clamp DPI a 12.750 (1.1 temporaneo), README + udev + LICENSE GPL-3.0 (3.3).
2. ✅ **Refactor** (completata): moduli `protocol`/`device`/`app`/`ui`, 9 test del protocollo con byte di riferimento, CI GitHub Actions, clippy pulito con `-D warnings`, commenti in inglese, ratatui 0.30 + crossterm 0.29 (3.2, 3.4). Restano da 3.4: `anyhow`/`thiserror` (insieme alla gestione errori della fase 4); da 3.3: metadati `Cargo.toml` e rinomina del pacchetto in `openvxe`.
3. ✅ **Reverse engineering** (completata):
   - fatto: `docs/PROTOCOL.md` con fonti e livelli di confidenza, `docs/CAPTURE.md`, `vxe-decode`, `vxe-probe` (`--info`, `--dump` in sola lettura, allowlist dei comandi), sniffer su file (`OPENVXE_SNIFF`);
   - ✅ verificato col mouse (`--info`, `--dump`): batteria `0x04`, CID/MID 2,31 (PAW3395SE), firmware 3.15, mappa EEPROM (stadi DPI, colori, tasti, debounce/motion sync/sleep); tasto avanti integro;
   - ✅ risolti 1.7, 1.13 e in parte 1.5 (un'apertura per path, solo interfacce vendor), regola udev rinominata `70-` (con `99-` l'ACL non veniva applicata);
   - ✅ scritture testate: `SetEEPROM` a `0x00` (polling 1000→500→1000 Hz) funziona e viene confermata con un eco; il pacchetto legacy a `0x70` viene confermato ma ignorato (1.12); EEPROM finale identica al backup;
   - ✅ scoperto che con il mouse in sleep il dongle non risponde e le scritture vanno perse: la TUI deve gestire "nessuna risposta";
   - il seguito (lettura config, codifica DPI, stadi DPI, fix 1.6 e 1.12) è nella fase 4.
4. ✅ **TUI sul protocollo reale** (completata):
   - nuovo modulo `config`: immagine EEPROM `0x00–0xBF`, settaggi letti dalle coppie `(v, 0x55−v)`, scrittura del blocco minimo che contiene il campo (stadi DPI a coppie da 8 byte, blocco prestazioni `0xA9` da 10 byte);
   - coda di richieste in `app`: identità e firmware, lettura della config all'avvio, batteria ogni 10 s, una richiesta alla volta con timeout 300 ms, conferma di ogni scrittura con l'eco, scritture sullo stesso blocco unite, rilettura del blocco se il mouse rifiuta;
   - mouse in sleep: la TUI mostra "Waiting for the mouse", riprova ogni secondo e invia le modifiche al risveglio; uscire con modifiche in sospeso chiede conferma;
   - settaggi: stadi DPI (codici PAW3395SE), numero di stadi, stadio attivo, polling 125–1000 Hz, LOD, debounce, motion sync, sleep, angle snapping, ripple control;
   - ✅ testata sull'hardware: polling 1000→500→1000 Hz dalla TUI, entrambe le scritture confermate dall'eco, EEPROM finale identica al backup (192 byte);
   - risolti 1.1 (per PAW3395SE), 1.2, 1.6, 1.12, 2.1, 2.2.
5. ✅ **Robustezza** (completata):
   - errori tipizzati con `thiserror` (`DeviceError`, `ConfigError`), `anyhow` in `openvxe` e `vxe-probe`;
   - invio solo come output report, errori di invio/lettura propagati (1.5);
   - hotplug: disconnessione rilevata, ri-scan ogni 2 s, config riletta alla riconnessione (1.8), ✅ testato staccando il dongle;
   - mouse capture rimosso, il testo del terminale si può di nuovo selezionare (1.9);
   - pacchetto rinominato `openvxe`, metadati `Cargo.toml` (3.3, 3.4).
6. **Feature utente** (prossima): profili (2.5), CLI (2.6), UX (2.7).
7. **Espansione**: rimappatura tasti/macro (2.3), altri modelli/chipset, multipiattaforma, pacchetti.

Ogni punto è un buon candidato per una PR separata e piccola.
