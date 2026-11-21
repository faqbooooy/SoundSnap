# SoundSnap для Windows

Сборка релиза проверена с Rust 1.90.0 на Windows x64. Установите Rust через
`rustup` и выполните команды из корня репозитория.

```powershell
rustup toolchain install 1.90.0-x86_64-pc-windows-gnu --profile minimal
cargo +1.90.0-x86_64-pc-windows-gnu build --release --locked --target x86_64-pc-windows-gnu
```

Готовый файл находится в `target/x86_64-pc-windows-gnu/release/SoundSnap.exe`.
Логотип встроен в EXE. Для запуска достаточно самого файла и системных компонентов Windows.

Список устройств и громкость используют Windows Core Audio. Смена выхода по
умолчанию опирается на недокументированный интерфейс `IPolicyConfig`.
