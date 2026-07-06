use crate::elf;
use crate::gdt;
use crate::memory::BootInfoFrameAllocator;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::arch::global_asm;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::task::{Context, Poll};
use spin::Mutex;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags, Size4KiB,
};
use x86_64::VirtAddr;

const USER_STACK_TOP: u64 = 0x0000_5000_0000_0000;
const USER_STACK_SIZE: u64 = 4096 * 8;

const PROCESS_STACK_REGION_TOP: u64 = 0x0000_6000_0000_0000;
const PROCESS_STACK_STRIDE: u64 = 4096 * 16;

const BG_STACK_REGION_TOP: u64 = 0x0000_7000_0000_0000;
const BG_STACK_STRIDE: u64 = 4096 * 16;

/// Quantos ticks de timer um processo em background roda de cada vez antes
/// de devolver o controle pro executor cooperativo (Fase 17) -- curto o
/// bastante pro shell continuar parecendo responsivo entre fatias.
const QUANTUM_TICKS: u32 = 3;

/// Snapshot completo de um processo parado: os 15 GPRs empilhados a mao
/// (stub de syscall/timer, ordem tem que bater com o push/pop em asm)
/// seguidos do frame que a propria CPU empilha ao entrar numa interrupcao
/// (rip/cs/rflags/rsp/ss). Uma troca de processo e so sobrescrever este
/// bloco no lugar -- a CPU nao sabe (nem precisa saber) que trocou.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TrapFrame {
    pub rax: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

struct Process {
    regs: TrapFrame,
    alive: bool,
}

/// Scheduler round-robin da Fase 14 -- so ativo durante `run_preemptive`
/// (demo isolada, ver plano). Fora dela e' `None` e o timer se comporta
/// como sempre (so EOI), sem afetar o modelo sincrono de `run_elf`.
static SCHEDULER: Mutex<Option<Vec<Process>>> = Mutex::new(None);
static CURRENT: AtomicUsize = AtomicUsize::new(0);

struct BgProcess {
    regs: TrapFrame,
    alive: bool,
    started: bool,
    exit_code: i64,
    name: String,
}

/// Processos "em background" da Fase 17 -- cada um roda em ring3 por uma
/// fatia curta (`QUANTUM_TICKS`) por vez, encaixado no executor cooperativo
/// via `BackgroundTask` (uma Task normal, ver abaixo), em vez de bloquear
/// o shell inteiro como `run_elf`/`run_preemptive` fazem.
static BACKGROUND: Mutex<BTreeMap<usize, BgProcess>> = Mutex::new(BTreeMap::new());
static NEXT_BG_PID: AtomicUsize = AtomicUsize::new(0);

/// `true` enquanto uma fatia de processo em background esta rodando de
/// verdade em ring3 -- so o `timer_dispatch` mexe nisso, pra saber se deve
/// contar ticks de quantum. `CURRENT_BG_PID` diz qual processo salvar
/// quando a fatia acabar.
static BG_ACTIVE: AtomicBool = AtomicBool::new(false);
static BG_TICK_COUNT: AtomicU32 = AtomicU32::new(0);
static CURRENT_BG_PID: AtomicUsize = AtomicUsize::new(0);

/// Mapper + frame allocator do kernel, guardados aqui pra shell::run_program
/// poder chegar neles -- Task/Executor exige futures `'static`, entao nao da
/// pra emprestar referencias locais de kernel_main; segue o mesmo padrao de
/// singleton global via Mutex ja usado por FS/PICS/TSS no resto do kernel.
pub static KERNEL_MEMORY: Mutex<Option<(OffsetPageTable<'static>, BootInfoFrameAllocator)>> =
    Mutex::new(None);

/// Carrega um ELF64 da FS e roda ele em ring3 ate ele sair (SYS_EXIT).
/// Sincrono: bloqueia quem chamou ate o processo terminar (sem preempcao,
/// um processo de usuario por vez -- coerente com o modelo cooperativo
/// ja existente no resto do kernel).
pub fn run_elf(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    bytes: &[u8],
) -> Result<i64, &'static str> {
    let entry = elf::load_elf(mapper, frame_allocator, bytes)?;
    let stack_top = map_user_stack(mapper, frame_allocator, USER_STACK_TOP, USER_STACK_SIZE)?;

    let cs = gdt::user_code_selector().0 as u64;
    let ss = gdt::user_data_selector().0 as u64;
    let rflags: u64 = 0x200; // IF=1, resto zerado

    let exit_code = unsafe { enter_usermode(entry, stack_top, cs, ss, rflags) };
    Ok(exit_code as i64)
}

/// Demo isolada da Fase 14: carrega varios ELFs e revezar eles de verdade
/// via timer (round-robin), ate todos saírem. Sincrono do ponto de vista
/// de quem chama -- por baixo, os processos passados se alternam pelo
/// timer em vez de rodar um de cada vez ate o fim.
pub fn run_preemptive(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    programs: &[&[u8]],
) -> Result<(), &'static str> {
    let cs = gdt::user_code_selector().0 as u64;
    let ss = gdt::user_data_selector().0 as u64;
    let rflags: u64 = 0x200;

    let mut processes = Vec::new();
    for (i, bytes) in programs.iter().enumerate() {
        let entry = elf::load_elf(mapper, frame_allocator, bytes)?;
        let stack_top = map_user_stack(
            mapper,
            frame_allocator,
            PROCESS_STACK_REGION_TOP - (i as u64) * PROCESS_STACK_STRIDE,
            USER_STACK_SIZE,
        )?;
        let regs = TrapFrame {
            rax: 0,
            rdi: 0,
            rsi: 0,
            rcx: 0,
            rdx: 0,
            rbx: 0,
            rbp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: entry,
            cs,
            rflags,
            rsp: stack_top,
            ss,
        };
        processes.push(Process { regs, alive: true });
    }

    let first = processes[0].regs;
    CURRENT.store(0, Ordering::Relaxed);
    *SCHEDULER.lock() = Some(processes);

    unsafe {
        enter_usermode(first.rip, first.rsp, first.cs, first.ss, first.rflags);
    }
    Ok(())
}

/// Carrega um ELF e registra ele como processo em background (Fase 17),
/// sem entrar em ring3 ainda -- so na primeira vez que `poll_background`
/// for chamado (pela `BackgroundTask` correspondente) ele de fato comeca
/// a rodar. Retorna o pid pra shell acompanhar.
pub fn spawn_background(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    bytes: &[u8],
    name: String,
) -> Result<usize, &'static str> {
    let entry = elf::load_elf(mapper, frame_allocator, bytes)?;
    let pid = NEXT_BG_PID.fetch_add(1, Ordering::Relaxed);
    let stack_top = map_user_stack(
        mapper,
        frame_allocator,
        BG_STACK_REGION_TOP - (pid as u64) * BG_STACK_STRIDE,
        USER_STACK_SIZE,
    )?;

    let cs = gdt::user_code_selector().0 as u64;
    let ss = gdt::user_data_selector().0 as u64;
    let rflags: u64 = 0x200;

    let regs = TrapFrame {
        rax: 0,
        rdi: 0,
        rsi: 0,
        rcx: 0,
        rdx: 0,
        rbx: 0,
        rbp: 0,
        r8: 0,
        r9: 0,
        r10: 0,
        r11: 0,
        r12: 0,
        r13: 0,
        r14: 0,
        r15: 0,
        rip: entry,
        cs,
        rflags,
        rsp: stack_top,
        ss,
    };

    BACKGROUND.lock().insert(
        pid,
        BgProcess {
            regs,
            alive: true,
            started: false,
            exit_code: 0,
            name,
        },
    );
    Ok(pid)
}

/// Lista (pid, nome) de todos os processos em background ainda vivos --
/// usado pelo comando `ps` do shell.
pub fn list_background() -> Vec<(usize, String)> {
    BACKGROUND
        .lock()
        .iter()
        .map(|(&pid, p)| (pid, p.name.clone()))
        .collect()
}

/// Remove um processo em background antes dele terminar sozinho -- usado
/// pelo comando `kill` do shell. So' e' seguro chamar isso fora da propria
/// fatia de execucao dele (nunca acontece por causa do modelo cooperativo:
/// um processo em background so roda em ring3 durante o `poll()` da SUA
/// PROPRIA `BackgroundTask`, nunca durante o de outra Task como o shell).
/// Retorna `true` se um processo com esse pid existia.
pub fn kill_background(pid: usize) -> bool {
    BACKGROUND.lock().remove(&pid).is_some()
}

pub enum BgPollResult {
    StillRunning,
    Exited(i64),
    NotFound,
}

/// Roda o processo em background `pid` por uma fatia (`QUANTUM_TICKS`)
/// ou ate ele sair de verdade, o que vier primeiro. Chamado pela
/// `BackgroundTask::poll` a cada vez que o executor cooperativo poll-a.
pub fn poll_background(pid: usize) -> BgPollResult {
    let (started, entry, stack, cs, ss, rflags) = {
        let mut map = BACKGROUND.lock();
        let proc = match map.get_mut(&pid) {
            Some(p) => p,
            None => return BgPollResult::NotFound,
        };
        let started = proc.started;
        proc.started = true;
        (
            started,
            proc.regs.rip,
            proc.regs.rsp,
            proc.regs.cs,
            proc.regs.ss,
            proc.regs.rflags,
        )
    };

    CURRENT_BG_PID.store(pid, Ordering::Relaxed);
    BG_TICK_COUNT.store(0, Ordering::Relaxed);
    BG_ACTIVE.store(true, Ordering::Relaxed);

    if started {
        // resume_process precisa do TrapFrame completo (todos os GPRs),
        // nao so os 5 campos escalares -- copia local, sem depender de
        // ponteiro pro mapa (que pode ser mexido por timer_dispatch
        // enquanto a fatia roda).
        let mut saved = BACKGROUND.lock().get(&pid).map(|p| p.regs);
        if let Some(regs) = saved.as_mut() {
            unsafe { resume_process(regs as *mut TrapFrame) };
        }
    } else {
        unsafe {
            enter_usermode(entry, stack, cs, ss, rflags);
        }
    }

    BG_ACTIVE.store(false, Ordering::Relaxed);

    let mut map = BACKGROUND.lock();
    match map.get(&pid) {
        Some(p) if p.alive => BgPollResult::StillRunning,
        Some(p) => {
            let code = p.exit_code;
            map.remove(&pid);
            BgPollResult::Exited(code)
        }
        None => BgPollResult::NotFound,
    }
}

/// Future que representa um processo em background (Fase 17) dentro do
/// executor cooperativo -- a cada `poll`, roda uma fatia via
/// `poll_background` e se recoloca na fila (`wake_by_ref`) se o processo
/// ainda estiver vivo, ao inves de guardar um waker (mais simples: o
/// processo "sempre tem trabalho", entao so pedir pra ser chamado nesse
/// mesmo ciclo/no proximo funciona igual bem).
pub struct BackgroundTask {
    pid: usize,
    name: String,
}

impl BackgroundTask {
    pub fn new(pid: usize, name: String) -> Self {
        BackgroundTask { pid, name }
    }
}

impl Future for BackgroundTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        match poll_background(self.pid) {
            BgPollResult::StillRunning => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            BgPollResult::Exited(code) => {
                crate::println!(
                    "processo em background '{}' (pid {}) saiu com codigo {}",
                    self.name,
                    self.pid,
                    code
                );
                Poll::Ready(())
            }
            BgPollResult::NotFound => Poll::Ready(()),
        }
    }
}

fn map_user_stack(
    mapper: &mut OffsetPageTable<'static>,
    frame_allocator: &mut impl FrameAllocator<Size4KiB>,
    top: u64,
    size: u64,
) -> Result<u64, &'static str> {
    let flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::USER_ACCESSIBLE
        | PageTableFlags::NO_EXECUTE;

    let stack_start = VirtAddr::new(top - size);
    let stack_end = VirtAddr::new(top - 1);
    let start_page = Page::<Size4KiB>::containing_address(stack_start);
    let end_page = Page::<Size4KiB>::containing_address(stack_end);

    for page in Page::range_inclusive(start_page, end_page) {
        if mapper.translate_page(page).is_err() {
            let frame = frame_allocator
                .allocate_frame()
                .ok_or("sem memoria fisica pra stack de usuario")?;
            unsafe {
                mapper
                    .map_to(page, frame, flags, frame_allocator)
                    .map_err(|_| "falha ao mapear stack de usuario")?
                    .flush();
            }
        }
    }

    Ok(top)
}

/// Chamado pelo novo `timer_entry_asm` a cada tick. Ordem de prioridade:
/// 1) demo isolada da Fase 14 (`SCHEDULER`, round-robin entre varios
///    processos) -- comportamento inalterado; 2) processo em background
///    da Fase 17 -- conta ticks, e ao bater o quantum salva o estado e
///    devolve o controle pra `poll_background` (sem o processo ter saido
///    de verdade); 3) nenhum dos dois -- so o EOI/heartbeat de sempre.
#[no_mangle]
extern "C" fn timer_dispatch(regs: &mut TrapFrame) {
    unsafe {
        crate::interrupts::PICS
            .lock()
            .notify_end_of_interrupt(crate::interrupts::PIC_1_OFFSET);
    }

    {
        let mut guard = SCHEDULER.lock();
        if let Some(processes) = guard.as_mut() {
            let current = CURRENT.load(Ordering::Relaxed);
            processes[current].regs = *regs;

            let n = processes.len();
            let mut next_idx = current;
            for step in 1..=n {
                let idx = (current + step) % n;
                if processes[idx].alive {
                    next_idx = idx;
                    break;
                }
            }
            CURRENT.store(next_idx, Ordering::Relaxed);
            *regs = processes[next_idx].regs;
            return;
        }
    }

    if BG_ACTIVE.load(Ordering::Relaxed) {
        let ticks = BG_TICK_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
        if ticks >= QUANTUM_TICKS {
            let pid = CURRENT_BG_PID.load(Ordering::Relaxed);
            if let Some(p) = BACKGROUND.lock().get_mut(&pid) {
                p.regs = *regs;
            }
            BG_ACTIVE.store(false, Ordering::Relaxed);
            // Diverge: devolve pra `poll_background`, que chamou
            // enter_usermode/resume_process -- o processo continua vivo,
            // so pausado (nao e' o mesmo caminho de SYS_EXIT de verdade).
            unsafe { exit_to_kernel(0) };
        }
        return;
    }

    // Fase 22: o heartbeat inundaria o console grafico (so' 12 linhas) em
    // questao de segundos -- sem utilidade nenhuma la, so' na VGA texto.
    if !crate::vga_buffer::is_gui_mode() {
        crate::print!(".");
    }
}

/// Chamado pelo dispatch de syscall (`src/syscall.rs`) quando `SYS_EXIT`
/// acontece. Ordem de prioridade (mesma do `timer_dispatch`): 1) demo da
/// Fase 14 -- troca pro proximo vivo se tiver, ou finaliza o scheduler;
/// 2) processo em background da Fase 17 saindo de verdade -- marca morto
/// + guarda o exit code, pra `poll_background` diferenciar de uma pausa
/// de quantum; 3) nenhum dos dois (modelo sincrono simples da Fase 13)
/// -- cai no `exit_to_kernel` de sempre.
pub fn scheduler_exit_current(regs: &mut TrapFrame, exit_code: u64) {
    {
        let mut guard = SCHEDULER.lock();
        if let Some(processes) = guard.as_mut() {
            let current = CURRENT.load(Ordering::Relaxed);
            processes[current].alive = false;

            let n = processes.len();
            let mut next_idx = None;
            for step in 1..=n {
                let idx = (current + step) % n;
                if processes[idx].alive {
                    next_idx = Some(idx);
                    break;
                }
            }
            let next_regs = next_idx.map(|idx| processes[idx].regs);

            match next_idx {
                Some(idx) => {
                    CURRENT.store(idx, Ordering::Relaxed);
                    *regs = next_regs.unwrap();
                    return;
                }
                None => {
                    *guard = None;
                    drop(guard);
                    unsafe { exit_to_kernel(exit_code) };
                }
            }
        }
    }

    if BG_ACTIVE.load(Ordering::Relaxed) {
        let pid = CURRENT_BG_PID.load(Ordering::Relaxed);
        if let Some(p) = BACKGROUND.lock().get_mut(&pid) {
            p.alive = false;
            p.exit_code = exit_code as i64;
        }
        BG_ACTIVE.store(false, Ordering::Relaxed);
        unsafe { exit_to_kernel(exit_code) };
    }

    unsafe { exit_to_kernel(exit_code) };
}

extern "C" {
    /// Salta pra ring3 (iretq) e so retorna quando o processo chamar
    /// SYS_EXIT (via `exit_to_kernel`, abaixo) -- nao segue o caminho
    /// normal do stub de syscall (`syscall_handler_asm`), que so serve
    /// pra syscalls que devem continuar rodando o mesmo processo (SYS_WRITE).
    fn enter_usermode(entry: u64, user_stack_top: u64, cs: u64, ss: u64, rflags: u64) -> u64;

    /// Chamado de dentro do handler de SYS_EXIT (src/syscall.rs) quando nao
    /// ha mais processo pra trocar. Restaura o rsp/rip do kernel salvos por
    /// `enter_usermode` -- um "longjmp" manual que faz `run_elf`/
    /// `run_preemptive` retornar de verdade em vez do processo travar num
    /// loop infinito (como na demo antiga da Fase 10).
    pub fn exit_to_kernel(code: u64) -> !;

    /// Ponto de entrada do timer (substitui o handler `extern "x86-interrupt"`
    /// de antes) -- empilha os mesmos 15 GPRs que `syscall_handler_asm`,
    /// chama `timer_dispatch`, despila e da `iretq`. Precisa ser asm puro
    /// (nao `extern "x86-interrupt"`) porque o compilador esconde os GPRs
    /// do codigo Rust nesse ABI, e a troca de processo precisa mexer neles.
    pub fn timer_entry_asm();

    /// Retoma um processo em background (Fase 17) a partir de um TrapFrame
    /// ja salvo (`regs_ptr`, 20 qwords) -- mesmo "setjmp" de
    /// `enter_usermode`, mas copia o frame salvo pra pilha em vez de montar
    /// um novo a partir de entry/stack/cs/ss/rflags. So retorna quando
    /// `exit_to_kernel` for chamado (fim de quantum ou saida de verdade).
    fn resume_process(regs_ptr: *mut TrapFrame) -> u64;
}

global_asm!(
    ".bss",
    ".align 8",
    "kernel_resume_rsp: .quad 0",
    "kernel_resume_rip: .quad 0",
    ".text",
    ".global enter_usermode",
    "enter_usermode:",
    // Callee-saved (System V ABI) -- precisam sobreviver ate o "retorno"
    // via exit_to_kernel, que so restaura rsp/rip, nao os registradores.
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "lea rax, [rip + kernel_resume_rsp]",
    "mov [rax], rsp",
    "lea rax, [rip + 2f]",
    "lea r9, [rip + kernel_resume_rip]",
    "mov [r9], rax",
    // Frame do iretq: ss, rsp, rflags, cs, rip (nessa ordem, empilhados
    // de baixo pra cima). Args (SysV): rdi=entry, rsi=stack, rdx=cs,
    // rcx=ss, r8=rflags.
    "push rcx",
    "push rsi",
    "push r8",
    "push rdx",
    "push rdi",
    "iretq",
    "2:",
    // So chega aqui via exit_to_kernel (jmp direto, nao um "ret" normal).
    // iretq nunca restaurou RFLAGS aqui (bypassed), entao IF ainda esta
    // zerado (setado pelo interrupt gate do int 0x80) -- reabilita.
    "sti",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    // rdi ainda tem o codigo de saida passado pra exit_to_kernel --
    // jmp preserva registradores, so muda rip.
    "mov rax, rdi",
    "ret",
    ".global exit_to_kernel",
    "exit_to_kernel:",
    "lea rax, [rip + kernel_resume_rsp]",
    "mov rsp, [rax]",
    "lea rax, [rip + kernel_resume_rip]",
    "jmp [rax]",
    // Empilha os mesmos 15 GPRs que syscall_handler_asm, na mesma ordem --
    // struct TrapFrame (acima) descreve o layout resultante. Ordem tem que
    // bater EXATAMENTE com o pop no final e com o que syscall_handler_asm
    // usa, ja que os dois alimentam a mesma struct/scheduler.
    ".global timer_entry_asm",
    "timer_entry_asm:",
    "push r15",
    "push r14",
    "push r13",
    "push r12",
    "push r11",
    "push r10",
    "push r9",
    "push r8",
    "push rbp",
    "push rbx",
    "push rdx",
    "push rcx",
    "push rsi",
    "push rdi",
    "push rax",
    "mov rdi, rsp",
    "call timer_dispatch",
    "jmp resume_trapframe_tail",
    // Despila os mesmos 15 GPRs (mesma ordem de `syscall_handler_asm`) e
    // da `iretq` -- mecanico, sem branch nenhum, entao e' seguro varias
    // rotinas pularem pra ca (timer_entry_asm depois do dispatch normal,
    // e resume_process depois de copiar um TrapFrame salvo pra pilha).
    "resume_trapframe_tail:",
    "pop rax",
    "pop rdi",
    "pop rsi",
    "pop rcx",
    "pop rdx",
    "pop rbx",
    "pop rbp",
    "pop r8",
    "pop r9",
    "pop r10",
    "pop r11",
    "pop r12",
    "pop r13",
    "pop r14",
    "pop r15",
    "iretq",
    // Retoma um processo em background (Fase 17) a partir de um TrapFrame
    // ja salvo em memoria (rdi = ponteiro). Mesmo "setjmp" de
    // enter_usermode (push callee-saved + salva rsp/rip de retorno), mas
    // em vez de montar o frame do iretq a partir de argumentos escalares,
    // copia as 20 qwords do TrapFrame pra pilha (rep movsq) e cai no
    // mesmo "pop tudo + iretq" que o timer usa. O retorno pro kernel (via
    // exit_to_kernel) usa seu proprio rotulo aqui embaixo em vez de
    // reusar o "2:" de enter_usermode -- evita depender de rotulos
    // numericos resolvendo a distancia certa dentro do mesmo bloco de asm.
    ".global resume_process",
    "resume_process:",
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "lea rax, [rip + kernel_resume_rsp]",
    "mov [rax], rsp",
    "lea rax, [rip + 3f]",
    "lea r9, [rip + kernel_resume_rip]",
    "mov [r9], rax",
    "mov rsi, rdi",
    "sub rsp, 160",
    "mov rdi, rsp",
    "mov rcx, 20",
    "rep movsq",
    "jmp resume_trapframe_tail",
    "3:",
    "sti",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    "mov rax, rdi",
    "ret",
);
