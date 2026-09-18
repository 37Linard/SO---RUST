# meu-os

Sistema operacional escrito do zero em Rust, alvo x86_64. Kernel bare-metal (`no_std`/`no_main`), sem depender de nenhum SO por baixo — boot direto via bootloader + QEMU.

## Features

- **Boot** — kernel `no_std` bootável, bootloader via crate [`bootloader`](https://github.com/rust-osdev/bootloader) 0.9.x
- **VGA text mode** — writer com macros `println!`/`print!`, cores, scroll (`src/vga_buffer.rs`)
- **GDT + IDT** — GDT com TSS/IST, handlers de breakpoint e double fault (`src/gdt.rs`, `src/interrupts.rs`)
- **PIC 8259 + interrupções de hardware** — timer e teclado (`src/interrupts.rs`)
- **Handler de page fault** — imprime CR2 (endereço que causou a falta) + código de erro decodificado antes de travar, em vez de virar double fault sem diagnóstico (`src/interrupts.rs`)
- **Paginação / memória virtual** — `OffsetPageTable` + `BootInfoFrameAllocator` sobre o memory map do bootloader (`src/memory.rs`)
- **Heap allocator** — `linked_list_allocator`, `Box`/`Vec`/`String` funcionando no kernel (`src/allocator.rs`)
- **Multitarefa cooperativa** — executor async/await (`Task`/`Executor`/`TaskWaker`), teclado como `Stream` assíncrono (`src/task/`)
- **Syscalls** — gate `int 0x80` (DPL Ring3), stub em assembly salva/restaura registradores, dispatch em Rust (`SYS_WRITE`, `SYS_EXIT`) (`src/syscall.rs`)
- **Filesystem persistente em disco** — driver ATA PIO (`src/ata.rs`, disco secundário `-hdb`) + filesystem próprio (superblock + diretório fixo + bump allocator, `src/fs.rs`) — sobrevive a reboot. API idêntica ao RAM-disk anterior (create/read/write/delete/list)
- **Userland real (ring3)** — salto genuíno de CPL0 para CPL3 via `iretq` manual, TSS.RSP0 configurado, páginas marcadas `USER_ACCESSIBLE` em todos os níveis da tabela de páginas (`src/process.rs`)
- **ELF loader** — parser ELF64 mínimo (`src/elf.rs`) mapeia segmentos `PT_LOAD` no próprio `p_vaddr` (sem relocação), shell roda binários reais da FS; contexto do kernel salvo/restaurado manualmente (`src/process.rs`) pra `SYS_EXIT` devolver o controle pro shell em vez de travar. Binário de teste em `programs/hello` (crate standalone, build separado)
- **Multitarefa preemptiva (demo)** — scheduler round-robin real (`src/process.rs`) troca de processo de verdade via timer (`TrapFrame` completo, 15 GPRs, empilhado por `timer_entry_asm`/`syscall_handler_asm`). Comando `preempt` no shell roda `programs/looper_a` + `programs/looper_b` concorrentemente, saída intercalada prova a troca real
- **Processos em background** — comando `spawn <arquivo>` roda um processo em fatias curtas (poucos ticks de timer por vez) encaixadas no executor cooperativo (`Task`/`Executor`, Fase 7), devolvendo o prompt na hora — shell continua respondendo ao teclado enquanto o processo roda (`src/process.rs`, `src/task/`). Comandos `ps`/`kill` listam e derrubam processos em background
- **Tela gráfica** — VGA modo 13h (320x200, 256 cores) via feature `vga_320x200` do bootloader, framebuffer linear (`src/framebuffer.rs`)
- **Mouse PS/2** — driver via controlador 8042, decodifica pacotes de 3 bytes (`src/mouse.rs`)
- **Compositor multi-janela** — N janelas (criar via "+" na barra de tarefas, arrastar, focar, minimizar, fechar), z-order real, backing-store em memória, cursor sem rastro (`src/desktop.rs`). Comando `gui` no shell ativa o modo gráfico
- **Fonte bitmap 8x8** — fonte própria (espaço, dígitos, maiúsculas, `+-:.$`), usada nos títulos das janelas e no botão "+" da barra de tarefas (`src/font8x8.rs`)
- **Shell dentro da janela gráfica** — `vga_buffer.rs` vira camada de redirecionamento (`print!`/`println!`/edição de linha) entre a VGA texto e um console gráfico novo (`src/gui_console.rs`) desenhado numa janela do compositor — o mesmo shell (`src/shell.rs`, sem nenhuma mudança de lógica) passa a rodar dentro da GUI assim que o comando `gui` liga o modo gráfico
- **Rede** — driver rtl8139 real (`src/rtl8139.rs`, PCI via `src/pci.rs`, polling sem interrupção) + Ethernet/ARP/IPv4/ICMP/UDP/TCP feitos à mão (`src/net/`) — comando `net` no shell responde ARP, ping (ICMP echo), servidor de eco UDP (porta 7777) e servidor de eco TCP (porta 7778, handshake + dados + fechamento real, uma conexão por vez), task assíncrona rodando junto com o shell (mesmo padrão de `process::BackgroundTask`)
- **Cliente DHCP** — DISCOVER/OFFER/REQUEST/ACK (RFC 2131) sobre broadcast, IP da interface passa a ser mutável (antes fixo em `10.0.2.15`) e é atualizado quando o lease chega (`src/net/dhcp.rs`). Comando `dhcp` no shell (precisa do `net` já iniciado) — sem renovação de lease/timeout/retransmissão, um pedido só
- **Edição de linha no shell** — setas esquerda/direita movem o cursor de verdade (cursor de hardware da VGA via CRTC), setas cima/baixo navegam o histórico de comandos (`src/shell.rs`, `src/vga_buffer.rs`)

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
qemu-system-x86_64 -drive format=raw,file=target\x86_64-meu_os\debug\bootimage-meu-os.bin -drive file=disk.img,format=raw,index=1,media=disk
```

Em Linux/macOS o processo é mais simples (linker do sistema já resolve): só `cargo bootimage` + `qemu-system-x86_64 -drive ...`.

### Disco persistente

O filesystem (`src/fs.rs`) precisa de um segundo disco pra guardar dados de verdade entre execuções — criar uma vez (`qemu-img` já vem junto com o QEMU):

```powershell
qemu-img create -f raw disk.img 16M
```

Depois é só sempre incluir `-drive file=disk.img,format=raw,index=1,media=disk` no comando do QEMU (como no exemplo acima) — os arquivos criados via `write` no shell sobrevivem a reboot enquanto o mesmo `disk.img` for reusado. Sem esse segundo `-drive`, o boot **trava** (não double fault) — o driver de disco (`src/ata.rs`) fica esperando por polling (`wait_not_busy`/`wait_data_ready`) um disco secundário que nunca responde.

### Binários de usuário (ELF loader)

`programs/hello` é um crate separado (fora do workspace do kernel) que compila pra um ELF64 real, executado pelo shell via `run_elf`. Precisa ser compilado **antes** do kernel, pois seu binário é embutido via `include_bytes!` em `main.rs`:

```powershell
cd programs\hello
cargo build --release
cd ..\..
cargo bootimage
```

No shell do meu-os, digite `hello` pra rodar o binário.

`programs/looper_a` e `programs/looper_b` (usados pelo comando `preempt`, demo de multitarefa preemptiva) seguem o mesmo esquema, mas com endereço base de linker diferente cada um (`--image-base`) — precisam ser compilados também antes do kernel:

```powershell
cd programs\looper_a; cargo build --release; cd ..\..
cd programs\looper_b; cargo build --release; cd ..\..
cargo bootimage
```

`programs/bgtask` (usado pelo comando `spawn`) também tem sua própria base de linker (`0x60000000`, distinta de tudo mais). No shell, `spawn bgtask` roda ele em background e devolve o prompt na hora.

### Rede

Precisa de uma placa de rede emulada (rtl8139) conectada ao QEMU:

```powershell
qemu-system-x86_64 -drive format=raw,file=target\x86_64-meu_os\debug\bootimage-meu-os.bin -drive file=disk.img,format=raw,index=1,media=disk -netdev user,id=net0 -device rtl8139,netdev=net0
```

No shell, `net` inicializa o driver (IP fixo `10.0.2.15` por padrão, dentro da faixa padrão do modo `user`/SLIRP do QEMU) e começa a responder ARP, ping (ICMP echo) e um servidor de eco UDP na porta 7777 em background. Rodando `dhcp` em seguida troca esse IP fixo por um de verdade, obtido do servidor DHCP do SLIRP.

**ICMP (ping)**: o modo `user`/SLIRP do QEMU não encaminha tráfego não solicitado do host pro guest (`ping 10.0.2.15` a partir do host não chega — nem chega a sair da rede real do host, já que `10.0.2.0/24` só existe dentro do processo do QEMU) — verificado com um "probe" que o próprio guest manda (ARP request pro gateway `10.0.2.2`, que responde de verdade) e com um auto-teste que injeta um echo request falso direto no responder, sem depender de rede real.

**UDP (eco)**: diferente do ICMP, dá pra testar de verdade a partir do host usando `hostfwd` (mapeia uma porta do host pra uma porta do guest):

```powershell
qemu-system-x86_64 ... -netdev user,id=net0,hostfwd=udp::7777-:7777 -device rtl8139,netdev=net0
```

Com o kernel rodando e `net` já iniciado, mandar um datagrama UDP pra `127.0.0.1:7777` do host faz o guest ecoar de volta o mesmo conteúdo — prova real de conectividade host↔guest, sem precisar de rede `tap`.

**TCP (eco)**: mesma ideia, com `hostfwd=tcp::7778-:7778`. Um `TcpClient`/`nc`/telnet real do host consegue abrir conexão de verdade (handshake completo), mandar dados (ecoados de volta) e fechar — servidor aceita várias conexões simultâneas (uma entrada por peer numa tabela, não mais um slot único), sem retransmissão/timeout (prova de conceito de handshake+dados+fechamento por conexão, não uma stack TCP completa). Testado com duas conexões concorrentes de verdade, cada uma ecoando seu próprio conteúdo sem misturar.

**DHCP**: com `net` já rodando, o comando `dhcp` manda um DISCOVER em broadcast; o servidor DHCP embutido no modo `user`/SLIRP do QEMU responde (OFFER → REQUEST → ACK) e o shell imprime o IP obtido (tipicamente `10.0.2.15`, a mesma faixa do IP fixo anterior). Testado direto pela porta serial (`serial_println!`), já que roda antes de qualquer inspeção visual.

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
├── elf.rs              parser ELF64, mapeia segmentos PT_LOAD
├── process.rs          salto real pra ring3 (CPL0 -> CPL3) + retorno via SYS_EXIT
├── ata.rs               driver ATA PIO (disco secundario, polling)
├── fs.rs               filesystem persistente em disco (superblock + diretorio + bump allocator)
├── framebuffer.rs      VGA modo 13h, primitivas de pixel
├── mouse.rs             driver PS/2
├── font8x8.rs           fonte bitmap 8x8 (espaco, digitos, maiusculas, +-:.$)
├── gui_console.rs       console de texto pra janela terminal (grade de caracteres)
├── desktop.rs           compositor multi-janela (backing-store, arrastar/focar/minimizar/fechar, janela terminal)
├── pci.rs               enumeracao de barramento PCI (portas 0xCF8/0xCFC)
├── rtl8139.rs           driver da placa de rede (PIO/DMA por polling, sem IRQ)
├── net/                 Ethernet, ARP, IPv4, ICMP, UDP, TCP, DHCP (serializacao manual) + task de polling
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

Roadmap original completo (boot → userland/ring3) + todos os itens extras planejados: tela gráfica, mouse, compositor multi-janela com títulos de texto real, fonte bitmap 8x8, shell rodando dentro da janela gráfica, shell interativo com edição de linha/histórico, ELF loader, demo de multitarefa preemptiva real, processos em background integrados ao shell com `ps`/`kill`, filesystem persistente em disco (ATA PIO), driver de rede (rtl8139) com ARP/ICMP/UDP/TCP (handshake + eco + fechamento reais, múltiplas conexões simultâneas) e cliente DHCP (DISCOVER/OFFER/REQUEST/ACK).

Ideias futuras: renovação de lease DHCP (hoje um pedido só, sem timeout/retransmissão), retransmissão/timeout de verdade no TCP (hoje sem RTO — segmento perdido fica sem resposta), limite/expiração de conexões TCP penduradas (hoje sem cap, um peer que nunca fecha ocupa a tabela pra sempre), rede `tap` pra testar ICMP de fora do host de verdade (UDP/TCP já dão, via `hostfwd`).

## Referências

- [Writing an OS in Rust](https://os.phil-opp.com/) — Philipp Oppermann
- [OSDev Wiki](https://wiki.osdev.org/)
- [xv6](https://github.com/mit-pdos/xv6-public) — teaching OS do MIT
