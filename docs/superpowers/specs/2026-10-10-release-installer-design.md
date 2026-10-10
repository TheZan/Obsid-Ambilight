# Установщик и выпуск релизов: дизайн

**Дата:** 2026-10-10
**Статус:** согласовано в обсуждении, ожидает просмотра спецификации
**Основа:** итоги Этапа 3 (`docs/testing.md`), новые значки (`assets/icons`)

## 1. Цель и границы

Пользователь скачивает установщик со страницы Releases на GitHub, ставит
приложение без прав администратора и обновляет его, запуская новый установщик
поверх. Релиз появляется сам при пуше тега `vX.Y.Z`.

**Входит:**

- корректное закрытие приложения по `WM_CLOSE` (нужно установщику);
- установщик Inno Setup 6 для текущего пользователя, RU/EN;
- CI: проверки на каждый пуш в `main` и pull request;
- выпуск релиза по тегу: проверки, сборка, установщик, `SHA256SUMS.txt`;
- README: установка из Releases, порядок выпуска версии.

**Не входит:** уведомление о новой версии (следующая отдельная задача),
автообновление, цифровая подпись, portable-сборка и `obsid-poc.exe` в релизе.

## 2. Корректное закрытие по `WM_CLOSE`

- Скрытое окно трея (`ObsidAmbilightTray`) сейчас не обрабатывает `WM_CLOSE`:
  `DefWindowProc` уничтожает окно, приложение выходит без `Command::Exit`, и
  лента в Ambilight застывает на последнем кадре (как в A13).
- `WM_CLOSE` → `begin_exit()`, как пункт «Выход» в меню: остановка стрима,
  действие при выходе по настройке, затем `finish_exit()`.
- Повторный `WM_CLOSE` ничего не меняет (`begin_exit` уже защищён флагом).
- Тест: окно трея с заглушкой worker'а получает `WM_CLOSE` → уходит
  `Command::Exit`. Если тест через окно окажется непрактичным, логика выносится в
  чистую функцию `ui::lifecycle`/`tray` и проверяется она.

## 3. Установщик

Файл `installer/obsid-ambilight.iss`, Inno Setup 6.

- `PrivilegesRequired=lowest`, `DefaultDirName={localappdata}\Programs\Obsid Ambilight`,
  без страниц выбора папки и группы меню.
- Языки: `English` и `Russian` (стандартные `.isl` Inno Setup), выбор по языку
  Windows.
- Постоянный `AppId` (GUID), чтобы обновление ставилось поверх.
- Версия: `GetStringFileInfo(exe, "ProductVersion")` из собранного
  `target\release\obsid-ambilight.exe` — совпадает с `Cargo.toml`.
- Значок установщика и удаления: `assets\icons\app.ico` /
  `{app}\obsid-ambilight.exe`.
- Ярлык в меню «Пуск» всегда; на рабочем столе — задача, по умолчанию выключена.
- Выходной файл: `ObsidAmbilight-Setup-<версия>.exe` в `target\installer\`.

### 3.1. Закрытие запущенного приложения

При установке (`PrepareToInstall`) и удалении (`InitializeUninstall`):

1. Найти окно класса `ObsidAmbilightTray`; если есть — `PostMessage(WM_CLOSE)`.
2. Ждать до 10 с, пока не исчезнет мьютекс `Local\ObsidAmbilight.Instance`
   (`CheckForMutexes`).
3. Не исчез — `taskkill /F /IM obsid-ambilight.exe` (лента может остаться в
   последнем состоянии — крайний случай).

Окно настроек закрывает сам фоновый процесс при выходе.

### 3.2. После установки

- `[Run]`: «Запустить Obsid Ambilight», флажок отмечен, `nowait postinstall
  skipifsilent`.
- Запись автозапуска приложение создаёт само при старте (`[startup] autostart`).

### 3.3. Удаление

- Удаляются файлы и ярлыки.
- Удаляются значения `ObsidAmbilight` в
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` и
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run`.
- Вопрос «Удалить также настройки и раскладку ленты?» (RU/EN), по умолчанию
  «Нет»; «Да» → удалить `%APPDATA%\ObsidAmbilight`. При тихом удалении — не
  удалять.

## 4. CI (GitHub Actions)

### 4.1. `.github/workflows/ci.yml`

- Триггеры: `push` в `main`, `pull_request`.
- `runs-on: windows-latest`; `actions/checkout`, `dtolnay/rust-toolchain@stable`
  (компоненты `rustfmt`, `clippy`), `Swatinem/rust-cache`.
- Шаги: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`.
- `permissions: contents: read`.

### 4.2. `.github/workflows/release.yml`

- Триггер: `push` тегов `v*`; `permissions: contents: write`.
- Шаги:
  1. Тег без `v` должен совпадать с `version` из `Cargo.toml` (`cargo metadata`),
     иначе ошибка с обоими значениями.
  2. `fmt`, `clippy`, `test` — как в `ci.yml`.
  3. `cargo build --release`.
  4. Inno Setup: если `iscc` нет — `choco install innosetup -y`; затем
     `iscc installer\obsid-ambilight.iss`.
  5. `SHA256SUMS.txt` с хэшем установщика (формат `sha256sum`).
  6. `gh release create <тег> <установщик> SHA256SUMS.txt --title "Obsid Ambilight <версия>" --generate-notes`.

## 5. README

- «Установка»: скачать `ObsidAmbilight-Setup-x.y.z.exe` из Releases; SmartScreen
  предупредит о неизвестном издателе (нет подписи) — «Подробнее → Выполнить в
  любом случае»; обновление — новый установщик поверх, настройки сохраняются;
  удаление — «Параметры → Приложения».
- «Для разработчиков»: сборка из исходников (`cargo build --release`), локальная
  сборка установщика (`iscc installer\obsid-ambilight.iss`), выпуск версии:
  поднять `version` в `Cargo.toml` → коммит → тег `vX.Y.Z` → пуш тега.

## 6. Тестирование

- Автотест на `WM_CLOSE` → `Command::Exit`; полный набор, `fmt`, `clippy`.
- Ручная проверка установщика на машине пользователя (Inno Setup ставится через
  `winget install JRSoftware.InnoSetup` с его разрешения):
  - чистая установка: ярлык, запуск, запись автозапуска на установленный exe;
  - установка поверх при работающем Ambilight: лента не застывает, приложение
    запускается снова, настройки на месте;
  - удаление: «Нет» — настройки остаются, «Да» — папка удалена; запись
    автозапуска удалена в обоих случаях.
- Workflow проверяются первым пушем и первым тегом — только по команде
  пользователя.

## 7. Риски

- Без подписи SmartScreen и часть антивирусов предупреждают о неизвестном
  издателе.
- Состав образа `windows-latest` меняется; установка Inno Setup через `choco`
  страхует от его отсутствия.
- Принудительное завершение (шаг 3 в 3.1) может оставить ленту в последнем
  состоянии — только если приложение не ответило за 10 с.
