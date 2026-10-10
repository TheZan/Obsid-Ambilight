# Installer and Release CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A tag `vX.Y.Z` pushed to GitHub produces a release with a per-user Inno Setup installer that closes the running app gracefully, installs or upgrades in place, and uninstalls cleanly.

**Architecture:** The tray window learns to exit gracefully on `WM_CLOSE`; the installer (`installer/obsid-ambilight.iss`) posts `WM_CLOSE` to that window and waits for the instance mutex before touching files. Two GitHub Actions workflows: `ci.yml` checks every push to `main` and every PR; `release.yml` checks, builds, packs and publishes on a tag.

**Tech Stack:** Rust 2024 / windows-sys (tray), Inno Setup 6 (Pascal script), GitHub Actions on `windows-latest` with PowerShell, `gh` CLI.

**Spec:** `docs/superpowers/specs/2026-10-10-release-installer-design.md`

## Global Constraints

- Install for the current user only: `PrivilegesRequired=lowest`, `{localappdata}\Programs\Obsid Ambilight`.
- Installer languages: English and Russian; the uninstall question defaults to «No».
- Version comes from `Cargo.toml`; the release fails when the tag (without `v`) differs from it.
- The release contains only `ObsidAmbilight-Setup-<version>.exe` and `SHA256SUMS.txt`.
- Third-party actions: only `actions/checkout`, `dtolnay/rust-toolchain`, `Swatinem/rust-cache`.
- Window class `ObsidAmbilightTray`, mutex `Local\ObsidAmbilight.Instance`, Run value `ObsidAmbilight` — existing names, do not rename.
- Commits: subject line only, no body, no `Co-Authored-By`. No push and no tags without the user's explicit request.
- Every task ends with `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green.

## Review Focus

- Upgrade while Ambilight is streaming: the strip must not freeze on the last frame; the app must start again after setup.
- App not running at all during install/uninstall: no 10 s wait, no error, no `taskkill` noise.
- App hung (does not exit in 10 s): setup still proceeds (forced kill), files are replaced.
- Uninstall with the settings window open: the window closes with the app, the exe is removed.
- Tag with a different version than `Cargo.toml`: the release job fails before building, with both values in the message.

---

### Task 1: Graceful exit on `WM_CLOSE`

**Files:**
- Modify: `src/ui/tray.rs` (wndproc match, around `WM_QUERYENDSESSION => 1,`)

**Interfaces:**
- Consumes: `TrayApp::begin_exit()` (existing; sends `Command::Exit`, arms the 2 s `TIMER_EXIT` fallback, guarded by `exiting`).
- Produces: the tray window closes the app like the «Exit» menu item when it receives `WM_CLOSE`; Task 2 relies on it.

No automated test: `wndproc` needs a real window, a worker and the user's config and registry (an end-to-end run would rewrite the user's autostart entry). Verification is the manual script in Step 3, which is the same call the installer makes.

- [ ] **Step 1: Handle `WM_CLOSE`**

In `wndproc`, next to `WM_QUERYENDSESSION => 1,`, add:

```rust
        // The installer closes a running copy this way; exit like the menu item.
        WM_CLOSE => {
            app.begin_exit();
            0
        }
```

Add `WM_CLOSE` to the existing `windows_sys::Win32::UI::WindowsAndMessaging::{…}` import list.

- [ ] **Step 2: Build and run the checks**

Run: `cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test`
Expected: no diffs, no warnings, all tests pass (251 + 3 ignored).

- [ ] **Step 3: Manual check with the release build**

Stop the running app, `cargo build --release`, start `target\release\obsid-ambilight.exe` in Ambilight mode, then run (PowerShell):

```powershell
Add-Type -Namespace W -Name U -MemberDefinition '[DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern System.IntPtr FindWindowW(string c, string n); [DllImport("user32.dll")] public static extern bool PostMessageW(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);'
$h = [W.U]::FindWindowW("ObsidAmbilightTray", [NullString]::Value)
[W.U]::PostMessageW($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
Start-Sleep 3
Get-Process obsid-ambilight -ErrorAction SilentlyContinue
Select-String -Path "$env:APPDATA\ObsidAmbilight\logs\*.log" -Pattern "exit requested" | Select-Object -Last 1
```

Expected: no `obsid-ambilight` process left; the log has a fresh «exit requested» line; the user confirms the strip left Ambilight per «When the app exits» (did not freeze on a frame).

- [ ] **Step 4: Commit**

```bash
git add src/ui/tray.rs
git commit -m "Exit gracefully when the tray window is asked to close"
```

---

### Task 2: Inno Setup installer

**Files:**
- Create: `installer/obsid-ambilight.iss`

**Interfaces:**
- Consumes: `target\release\obsid-ambilight.exe` with `ProductVersion` (from `build.rs`/winresource), `assets\icons\app.ico`, Task 1's `WM_CLOSE` handling.
- Produces: `target\installer\ObsidAmbilight-Setup-<version>.exe`; Task 3 runs `iscc installer\obsid-ambilight.iss` and uploads that file.

- [ ] **Step 1: Install Inno Setup locally (ask the user first)**

Ask the user for permission, then run: `winget install --id JRSoftware.InnoSetup -e --scope user`
Expected: `ISCC.exe` exists under `%LOCALAPPDATA%\Programs\Inno Setup 6\` (or `C:\Program Files (x86)\Inno Setup 6\`).

- [ ] **Step 2: Write the script**

```iss
; Obsid Ambilight installer: per-user, no admin rights.
; Build: cargo build --release, then iscc installer\obsid-ambilight.iss

#define AppName "Obsid Ambilight"
#define AppExe "obsid-ambilight.exe"
#define Root SourcePath + "\.."
#define ExePath Root + "\target\release\" + AppExe

#if !FileExists(ExePath)
  #error Build the app first: cargo build --release
#endif

#define AppVersion GetStringFileInfo(ExePath, "ProductVersion")

[Setup]
AppId={{6B0D7F3E-2C4A-4E8B-9F1D-3A5C7E9B1D24}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisherURL=https://github.com/TheZan/Obsid-Ambilight
AppSupportURL=https://github.com/TheZan/Obsid-Ambilight/issues
AppUpdatesURL=https://github.com/TheZan/Obsid-Ambilight/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#Root}\target\installer
OutputBaseFilename=ObsidAmbilight-Setup-{#AppVersion}
SetupIconFile={#Root}\assets\icons\app.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2
SolidCompression=yes
CloseApplications=no
ShowLanguageDialog=no
LanguageDetectionMethod=uilanguage

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
en.RemoveSettings=Also remove the settings and the strip layout?
ru.RemoveSettings=Удалить также настройки и раскладку ленты?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
const
  WM_CLOSE = $0010;
  TrayClass = 'ObsidAmbilightTray';
  InstanceMutex = 'Local\ObsidAmbilight.Instance';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';
  RunValue = 'ObsidAmbilight';

{ Asks a running copy to exit (it turns the strip off or keeps the colour, as
  set by the user), waits up to 10 s, then kills whatever is left. }
procedure CloseRunningApp;
var
  Wnd: HWND;
  I, Code: Integer;
begin
  Wnd := FindWindowByClassName(TrayClass);
  if Wnd <> 0 then
    PostMessage(Wnd, WM_CLOSE, 0, 0);
  for I := 1 to 100 do
  begin
    if not CheckForMutexes(InstanceMutex) then
      Break;
    Sleep(100);
  end;
  if CheckForMutexes(InstanceMutex) or (Wnd <> 0) then
  begin
    { Give the process a moment to unload, then remove stragglers such as an
      open settings window. taskkill fails harmlessly when nothing is left. }
    Sleep(300);
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE,
      ewWaitUntilTerminated, Code);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningApp;
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  CloseRunningApp;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    RegDeleteValue(HKCU, RunKey, RunValue);
    RegDeleteValue(HKCU, ApprovedKey, RunValue);
  end;
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    if MsgBox(CustomMessage('RemoveSettings'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{userappdata}\ObsidAmbilight'), True, True, True);
end;
```

- [ ] **Step 3: Compile**

Run: `cargo build --release` then `& "<ISCC.exe path>" installer\obsid-ambilight.iss`
Expected: `Successful compile`, file `target\installer\ObsidAmbilight-Setup-0.1.0.exe`.

- [ ] **Step 4: Manual checks with the user (installer on their machine)**

1. App from `target\release` running in Ambilight → run the setup (Russian UI expected): the strip leaves Ambilight per the exit setting instead of freezing, setup finishes, «Запустить Obsid Ambilight» starts the installed copy; Run value now points to `%LOCALAPPDATA%\Programs\Obsid Ambilight\obsid-ambilight.exe`; Start menu has the shortcut; settings and layout are the same.
2. Run the same setup again over the installed copy while it runs → upgrade in place, app restarts, settings kept.
3. Settings → Apps → Obsid Ambilight → Uninstall, answer «Нет» → app gone, Run value gone, `%APPDATA%\ObsidAmbilight` still there.
4. Reinstall, uninstall, answer «Да» → `%APPDATA%\ObsidAmbilight` removed. (Ask before this step: it deletes the user's layout; back up `config.toml` to the scratchpad first and restore it afterwards.)

Expected: every item as described; record results in the ledger.

- [ ] **Step 5: Commit**

```bash
git add installer/obsid-ambilight.iss
git commit -m "Add the Inno Setup installer"
```

---

### Task 3: GitHub Actions workflows

**Files:**
- Create: `.github/workflows/ci.yml`
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: `installer/obsid-ambilight.iss` and its output path `target\installer\ObsidAmbilight-Setup-<version>.exe` (Task 2).
- Produces: nothing later tasks call; README (Task 4) documents the release steps.

- [ ] **Step 1: Write `ci.yml`**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

jobs:
  check:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
```

- [ ] **Step 2: Write `release.yml`**

```yaml
name: Release

on:
  push:
    tags: ['v*']

permissions:
  contents: write

jobs:
  release:
    runs-on: windows-latest
    defaults:
      run:
        shell: pwsh
    env:
      TAG: ${{ github.ref_name }}
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2

      - name: Check that the tag matches Cargo.toml
        run: |
          $version = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages[0].version
          if ($env:TAG.TrimStart('v') -ne $version) {
            throw "Tag $env:TAG does not match version $version in Cargo.toml"
          }
          "VERSION=$version" >> $env:GITHUB_ENV

      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
      - run: cargo build --release

      - name: Build the installer
        run: |
          $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
            Where-Object { Test-Path $_ } | Select-Object -First 1
          if (-not $iscc) {
            choco install innosetup -y --no-progress
            $iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
              Where-Object { Test-Path $_ } | Select-Object -First 1
          }
          & $iscc installer\obsid-ambilight.iss
          if ($LASTEXITCODE) { exit $LASTEXITCODE }

      - name: Write checksums
        run: |
          $name = "ObsidAmbilight-Setup-$env:VERSION.exe"
          $hash = (Get-FileHash "target\installer\$name" -Algorithm SHA256).Hash.ToLower()
          "$hash  $name" | Out-File target\installer\SHA256SUMS.txt -Encoding ascii

      - name: Publish the release
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          gh release create $env:TAG `
            "target\installer\ObsidAmbilight-Setup-$env:VERSION.exe" `
            target\installer\SHA256SUMS.txt `
            --verify-tag --title "Obsid Ambilight $env:VERSION" --generate-notes
```

- [ ] **Step 3: Dry-run the release steps locally**

Run in PowerShell from the repo root, with `$env:TAG = 'v0.1.0'` and then `$env:TAG = 'v9.9.9'`, the body of «Check that the tag matches Cargo.toml» (without the `GITHUB_ENV` line), then «Write checksums» with `$env:VERSION = '0.1.0'`.
Expected: `v0.1.0` passes; `v9.9.9` throws «Tag v9.9.9 does not match version 0.1.0 in Cargo.toml»; `target\installer\SHA256SUMS.txt` holds one line `<64 hex>  ObsidAmbilight-Setup-0.1.0.exe`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/ci.yml .github/workflows/release.yml
git commit -m "Add CI checks and tag-triggered releases"
```

---

### Task 4: README — installation and releasing

**Files:**
- Modify: `README.md` (`## Installation`, `## For developers`)
- Modify: `README.ru.md` (`## Установка`, `## Для разработчиков`)

**Interfaces:**
- Consumes: asset name `ObsidAmbilight-Setup-<version>.exe` (Task 2), the release flow (Task 3).
- Produces: user documentation only.

- [ ] **Step 1: Replace «Installation» in `README.md`**

```markdown
## Installation

Download `ObsidAmbilight-Setup-x.y.z.exe` from
[Releases](https://github.com/TheZan/Obsid-Ambilight/releases/latest) and run
it. It installs for your Windows account only and does not ask for administrator
rights.

The installer is not digitally signed, so Windows SmartScreen may warn about an
unknown publisher. Click **More info → Run anyway**.

To update, run the installer of the new version. It closes the running app,
replaces it and starts it again; your settings and strip layout stay. To remove
the app, use **Settings → Apps → Installed apps**; it asks whether to delete the
settings as well.
```

- [ ] **Step 2: Extend «For developers» in `README.md`**

Insert before the `cargo test` block:

```markdown
Build from source with [Rust](https://rustup.rs) 1.95 or newer:

```bash
cargo build --release                         # target/release/obsid-ambilight.exe
iscc installer/obsid-ambilight.iss            # target/installer/ObsidAmbilight-Setup-<version>.exe (Inno Setup 6)
```
```

and after the list of docs links:

```markdown
To release a version: set `version` in `Cargo.toml`, commit, then push a tag
`vX.Y.Z` with the same number. GitHub Actions checks the code, builds the
installer and publishes it on the Releases page.
```

- [ ] **Step 3: Same in `README.ru.md`**

```markdown
## Установка

Скачайте `ObsidAmbilight-Setup-x.y.z.exe` со страницы
[Releases](https://github.com/TheZan/Obsid-Ambilight/releases/latest) и
запустите. Приложение ставится только для вашей учётной записи Windows, права
администратора не нужны.

У установщика нет цифровой подписи, поэтому Windows SmartScreen может
предупредить о неизвестном издателе. Нажмите **Подробнее → Выполнить в любом
случае**.

Чтобы обновиться, запустите установщик новой версии: он закроет работающее
приложение, заменит его и запустит снова; настройки и раскладка ленты
сохранятся. Удалить приложение можно в **Параметры → Приложения → Установленные
приложения**; при удалении оно спросит, удалять ли и настройки.
```

«Для разработчиков» — перед блоком `cargo test`:

```markdown
Сборка из исходников — [Rust](https://rustup.rs) 1.95 или новее:

```bash
cargo build --release                         # target/release/obsid-ambilight.exe
iscc installer/obsid-ambilight.iss            # target/installer/ObsidAmbilight-Setup-<версия>.exe (Inno Setup 6)
```
```

после ссылок на документы:

```markdown
Выпуск версии: поменяйте `version` в `Cargo.toml`, закоммитьте и запушьте тег
`vX.Y.Z` с тем же номером. GitHub Actions проверит код, соберёт установщик и
выложит его на странице Releases.
```

- [ ] **Step 4: Check and commit**

Run: `grep -n "There is no installer\|Установщика нет" README.md README.ru.md`
Expected: no output.

```bash
git add README.md README.ru.md
git commit -m "Document installing from Releases and cutting a release"
```
