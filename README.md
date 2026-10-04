# LexiCards

A Windows app for studying English vocabulary, idioms, phrasal verbs, and language patterns. Built with Rust. Includes scheduled reviews, progress tracking, and local backups.

Choose your translation language on first launch. Definitions and examples stay in English. New dictionary lookups and translations need internet access; saved content works offline.

## Installation

Download `LexiCards.exe` from the [latest release](https://github.com/ibrahimgns1/lexicards/releases/latest) and run it on Windows.

To build from source, install [Rust](https://rustup.rs/) and the **Desktop development with C++** workload from [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).

```powershell
git clone https://github.com/ibrahimgns1/lexicards.git
cd lexicards
cargo build --locked --release
.\target\release\lexicards.exe
```

Your library is saved in `%LOCALAPPDATA%\LexiCards\LexiCards\data`.
