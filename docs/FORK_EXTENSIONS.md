# Estensioni & Integrazioni del Fork Coucou

Questo documento descrive le funzionalità introdotte in questo fork rispetto al repository upstream originale ([`Louis-CFM/coucou`](https://github.com/Louis-CFM/coucou)), la loro architettura tecnica e la strategia per mantenerle aggiornate nel tempo con facilità.

---

## 1. Funzionalità Aggiunte

Rispetto al branch `main` di upstream, questo fork introduce:

1. **Integrazione Completa con Feishin (`integration_feishin`)**:
   - Pillola dedicata nella barra Coucou (colore `#3574FC`).
   - Card **Now Playing** interattiva per visualizzare brano, artista, album, copertina ad alta risoluzione e barra di avanzamento temporale.
   - Controlli multimediali completi: *Play / Pausa*, *Brano successivo / precedente*, *Ripeti (Repeat)*, *Casuale (Shuffle)*, *Seek temporale* e *Regolazione volume*.
   - **Mochi Danzante**: Mochi balla sincronizzato a 112 BPM sia nell'isola che sul desktop mentre la musica è in riproduzione.
   - Supporto bivalente:
     - **Linux**: Comunicazione nativa via **D-Bus MPRIS v2** (`org.mpris.MediaPlayer2.Feishin`) nel backend Rust (`feishin.rs`). Zero configurazioni necessarie.
     - **Windows**: Comunicazione diretta via **Remote WebSocket** (`ws://127.0.0.1:4333`) nel frontend webview (`island/feishin.ts`). Non richiede fork o modifiche a Feishin.

2. **Supporto Nativo alla Lingua Italiana (`it`)**:
   - Diventa l'11ª lingua ufficiale supportata in Coucou (dopo en, zh-Hans, hi, es, ar, fr, bn, pt-BR, ru, id).
   - Traduzione integrale dei cataloghi stringhe (379 stringhe macOS/iOS in `Localizable.xcstrings`, 300 stringhe Windows/Linux in `extra.json`).
   - Rilevamento automatico della lingua di sistema (`it`, `it-IT`, `it-CH`) su Linux, Windows e macOS.
   - Traduzione completa dell'installer Windows (NSIS).

3. **Workflow di Release Automatizzato (GitHub Actions)**:
   - Compilazione e pubblicazione automatica dei pacchetti Linux (`.rpm`, `.deb`, `.AppImage`) e Windows (`.exe`, `.msi`) nella sezione **Releases** di GitHub tramite Git Tags (`linux-v*` / `windows-v*`) o con un solo clic via interfaccia web (*Actions → Run workflow*).
   - Progressione corretta delle versioni (da `0.3.0` a `0.3.1+`) per garantire la compatibilità con i gestori di pacchetti come `sudo dnf upgrade`.

---

## 2. Architettura & Perché è Facile da Mantenere ("Zero-Fork")

L'integrazione con Feishin è stata ingegnerizzata appositamente per richiedere **il minor intervento umano possibile** durante i futuri aggiornamenti di Coucou o di Feishin:

- **Isolamento dei file**:
  - Tutta la logica di Feishin è racchiusa in file dedicati:
    - Backend Rust Linux: [`windows/src-tauri/src/feishin.rs`](../windows/src-tauri/src/feishin.rs)
    - Frontend TypeScript: [`windows/src/island/feishin.ts`](../windows/src/island/feishin.ts), [`windows/src/views/feishin.ts`](../windows/src/views/feishin.ts), [`windows/src/core/feishin.ts`](../windows/src/core/feishin.ts)
  - Nei file core di Coucou sono state aggiunte solo minime chiamate di registrazione (registrazione pillola, evento di ascolto). Questo azzera quasi del tutto il rischio di conflitti di merge (`git merge conflicts`) quando upstream aggiorna il core.

- **Standard Aperti e Stabili**:
  - Su Linux sfrutta lo standard di sistema **MPRIS v2** (FreeDesktop D-Bus): qualsiasi versione futura di Feishin o di player MPRIS continuerà a funzionare senza modifiche al codice.
  - Su Windows sfrutta le API del server Remote WebSocket integrato in Feishin, senza richiedere fork di Feishin o patch esterne.

---

## 3. Guida alla Creazione delle Release su GitHub

Puoi generare automaticamente i pacchetti compilati e farli apparire nella sezione **Releases** del tuo repository GitHub in due modi:

### Metodo A: Tramite Git Tag (Metodo Standard Ufficiale)
Quando sei pronto a rilasciare una versione (es. `0.3.1`):
```bash
# Per Linux:
git tag linux-v0.3.1
git push origin linux-v0.3.1

# Per Windows:
git tag windows-v0.3.1
git push origin windows-v0.3.1
```
Il workflow di GitHub Actions:
1. Rileverà il tag.
2. Compilerà tutti i pacchetti.
3. Creerà automaticamente una **GitHub Release** allegando i file (`.rpm`, `.deb`, `.AppImage` o `.exe`, `.msi`).

### Metodo B: Dall'interfaccia GitHub (Run Workflow con Release)
Nei file workflow `.github/workflows/linux.yml` e `windows.yml` abbiamo aggiunto il parametro `publish_release`:
1. Vai su **Actions** su GitHub.
2. Clicca su **Linux** o **Windows**.
3. Clicca su **Run workflow**.
4. Spunta la casella **"Publish as GitHub Release?"**.
5. Clicca **Run workflow**: GitHub Actions compilerà e pubblicherà la release direttamente con i file pronti per il download.

---

## 4. Come Sincronizzare il Fork con Upstream in Futuro

Quando il repository originale (`Louis-CFM/coucou`) rilascia nuove funzionalità o aggiornamenti:

### Passo 1: Sincronizzare il ramo `main`
Puoi farlo con il pulsante **"Sync fork"** su GitHub oppure da terminale:
```bash
git checkout main
git fetch upstream
git merge upstream/main
git push origin main
```

### Passo 2: Integrare gli aggiornamenti nel ramo delle nostre feature
```bash
git checkout feat/feishin-integration
git merge main
```
Poiché i file di Feishin e dell'italiano sono isolati o aggiunti in append, il merge sarà automatico ed esente da conflitti nella stragrande maggioranza dei casi.

### Passo 3: Incrementare la versione per una nuova release
Quando vuoi rilasciare una nuova build (es. `0.3.2`):
1. Aggiorna la versione in:
   - `windows/package.json`
   - `windows/src-tauri/tauri.conf.json`
   - `windows/Cargo.toml`
2. Esegui il commit e pusha il nuovo tag:
   ```bash
   git commit -am "chore: bump version to 0.3.2"
   git tag linux-v0.3.2
   git push origin feat/feishin-integration --tags
   ```
   `dnf upgrade` sui sistemi Linux rileverà immediatamente il nuovo pacchetto RPM e lo aggiornerà in automatico.
