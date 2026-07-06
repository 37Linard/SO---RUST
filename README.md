# meu-os

Sistema operacional escrito do zero em Rust, alvo x86_64. Kernel bare-metal (`no_std`/`no_main`), sem depender de nenhum SO por baixo — boot direto via bootloader + QEMU.

## Features

- **Boot** — kernel `no_std` bootável, bootloader via crate [`bootloader`](https://github.com/rust-osdev/bootloader) 0.9.x
- **VGA text mode** — writer com macros `println!`/`print!`, cores, scroll (`src/vga_buffer.rs`)
- **GDT + IDT** — GDT com TSS/IST, handlers de breakpoint e double fault (`src/gdt.rs`, `src/interrupts.rs`)
- **PIC 8259 + interrupções de hardware** — timer e teclado (`src/interrupts.rs`)
- **Paginação / memória virtual** — `OffsetPageTable` + `BootInfoFrameAllocator` sobre o memory map do bootloader (`src/memory.rs`)
- **Heap allocator** — `linked_list_allocator`, `Box`/`Vec`/`String` funcionando no kernel (`src/allocator.rs`)
- **Multitarefa cooperativa** — executor async/await (`Task`/`Executor`/`TaskWaker`), teclado como `Stream` assíncrono (`src/task/`)
- **Syscalls** — gate `int 0x80` (DPL Ring3), stub em assembly salva/restaura registradores, dispatch em Rust (`SYS_WRITE`, `SYS_EXIT`) (`src/syscall.rs`)
- **Filesystem em memória** — VFS simples (RAM-disk) com create/read/write/delete/list (`src/fs.rs`)
- **Userland real (ring3)** — salto genuíno de CPL0 para CPL3 via `iretq` manual, TSS.RSP0 configurado, páginas marcadas `USER_ACCESSIBLE` em todos os níveis da tabela de páginas (`src/usermode.rs`)
- **Tela gráfica** — VGA modo 13h (320x200, 256 cores) via feature `vga_320x200` do bootloader, framebuffer linear (`src/framebuffer.rs`)
- **Mouse PS/2** — driver via controlador 8042, decodifica pacotes de 3 bytes (`src/mouse.rs`)
- **Compositor de janelas básico** — backing-store em memória, janela arrastável pela barra de título, cursor sem rastro (`src/desktop.rs`)

## Requisitos

- **Rust nightly** com componentes `rust-src` e `llvm-tools-preview`
- **QEMU** (`qemu-system-x86_64`)
- **Windows**: Visual Studio Build Tools (workload C++) — necessário pro linker MSVC, mesmo pra ferramentas host como `bootimage`
- **bootimage**: `cargo install bootimage`

```powershell
rustup toolchain install nightly
rustup component add rust-src llvm-tools-preview --toolchain nightly
cargo install bootimage
```

## Build & Run

No Windows, o ambiente MSVC (`PATH`/`INCLUDE`/`LIB`) precisa ser configurado manualmente antes de compilar (o `vcvarsall.bat` pode falhar silenciosamente sem o `vswhere.exe`, que não vem no Build Tools mínimo):

```powershell
$msvcVer = "14.44.35207"   # ajustar pra versao instalada
$sdkVer = "10.0.26100.0"   # ajustar pra versao instalada
$vcRoot = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\$msvcVer"
$sdkRoot = "C:\Program Files (x86)\Windows Kits\10"

$env:PATH = "$vcRoot\bin\Hostx64\x64;$sdkRoot\bin\$sdkVer\x64;C:\Program Files\qemu;C:\Users\User\.cargo\bin;$env:PATH"
$env:INCLUDE = "$vcRoot\include;$sdkRoot\Include\$sdkVer\ucrt;$sdkRoot\Include\$sdkVer\shared;$sdkRoot\Include\$sdkVer\um;$sdkRoot\Include\$sdkVer\winrt"
$env:LIB = "$vcRoot\lib\x64;$sdkRoot\Lib\$sdkVer\ucrt\x64;$sdkRoot\Lib\$sdkVer\um\x64"

cargo bootimage
qemu-system-x86_64 -drive format=raw,file=target\x86_64-meu_os\debug\bootimage-meu-os.bin
```

Em Linux/macOS o processo é mais simples (linker do sistema já resolve): só `cargo bootimage` + `qemu-system-x86_64 -drive ...`.

## Arquitetura

```
src/
├── main.rs          entry point (bootloader::entry_point!), sequencia de boot e demos
├── vga_buffer.rs     writer VGA texto (println!/print!)
├── gdt.rs            GDT, TSS, seletores de segmento (kernel + usuario)
├── interrupts.rs     IDT, PIC 8259, handlers de interrupcao
├── memory.rs         paginacao, frame allocator
├── allocator.rs      heap allocator (linked_list_allocator)
├── syscall.rs         gate int 0x80, dispatch de syscalls
├── task/              executor async (multitarefa cooperativa)
│   ├── mod.rs
│   ├── executor.rs
│   └── keyboard.rs    teclado como Stream assincrono
├── usermode.rs        salto real pra ring3 (CPL0 -> CPL3)
├── fs.rs               filesystem em memoria (RAM-disk)
├── framebuffer.rs      VGA modo 13h, primitivas de pixel
├── mouse.rs             driver PS/2
├── desktop.rs           compositor (backing-store, janela arrastavel)
└── serial.rs            saida serial (debug via porta COM1)
```

## Debug

Pra diagnosticar problemas sem depender só de inspeção visual da janela do QEMU:

```powershell
qemu-system-x86_64 -drive format=raw,file=<imagem> -serial file:serial.log -monitor tcp:127.0.0.1:4444,server,nowait
```

- `serial.log` recebe saída de `serial_println!` (útil pra confirmar progresso mesmo quando o modo gráfico torna o texto VGA invisível)
- O monitor (porta 4444) aceita comando `screendump arquivo.ppm` — captura o framebuffer atual, útil pra inspecionar visualmente sem tirar screenshot manual

## Status

Roadmap original completo (boot → userland/ring3). Extras: tela gráfica, mouse, compositor básico de janelas.

Ideias futuras: multitarefa preemptiva real, loader de ELF, driver de disco (ATA/AHCI) pra filesystem persistente, rede, compositor com múltiplas janelas.

## Referências

- [Writing an OS in Rust](https://os.phil-opp.com/) — Philipp Oppermann
- [OSDev Wiki](https://wiki.osdev.org/)
- [xv6](https://github.com/mit-pdos/xv6-public) — teaching OS do MIT
