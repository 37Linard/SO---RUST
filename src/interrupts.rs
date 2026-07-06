use crate::gdt;
use crate::println;
use lazy_static::lazy_static;
use pic8259::ChainedPics;
use spin::Mutex;
use x86_64::registers::control::Cr2;
use x86_64::structures::idt::{InterruptDescriptorTable, InterruptStackFrame, PageFaultErrorCode};

pub const PIC_1_OFFSET: u8 = 32;
pub const PIC_2_OFFSET: u8 = PIC_1_OFFSET + 8;

pub static PICS: Mutex<ChainedPics> =
    Mutex::new(unsafe { ChainedPics::new(PIC_1_OFFSET, PIC_2_OFFSET) });

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InterruptIndex {
    Timer = PIC_1_OFFSET,
    Keyboard,
    Mouse = PIC_2_OFFSET + 4,
    /// IRQ14 (ATA primario) -- o controlador de disco dispara isso sozinho
    /// a cada comando PIO completado, MESMO rodando so por polling
    /// (`src/ata.rs` nunca espera por interrupcao). Sem handler pra esse
    /// vetor, a primeira leitura/escrita de disco vira double fault na
    /// hora (Fase 19) -- so precisa de EOI, nao faz nada com o disco aqui.
    PrimaryAta = PIC_2_OFFSET + 6,
}

impl InterruptIndex {
    fn as_u8(self) -> u8 {
        self as u8
    }

    fn as_usize(self) -> usize {
        usize::from(self.as_u8())
    }
}

lazy_static! {
    static ref IDT: InterruptDescriptorTable = {
        let mut idt = InterruptDescriptorTable::new();
        idt.breakpoint.set_handler_fn(breakpoint_handler);
        unsafe {
            idt.double_fault
                .set_handler_fn(double_fault_handler)
                .set_stack_index(gdt::DOUBLE_FAULT_IST_INDEX);
        }
        idt.page_fault.set_handler_fn(page_fault_handler);
        unsafe {
            // Precisa ser asm puro (nao "extern x86-interrupt"): o timer
            // agora pode trocar de processo (Fase 14), e o ABI de interrupt
            // handler do Rust esconde os GPRs do nosso codigo, impedindo
            // qualquer troca de contexto real.
            idt[InterruptIndex::Timer.as_usize()].set_handler_addr(x86_64::VirtAddr::new(
                crate::process::timer_entry_asm as *const () as u64,
            ));
        }
        idt[InterruptIndex::Keyboard.as_usize()].set_handler_fn(keyboard_interrupt_handler);
        idt[InterruptIndex::Mouse.as_usize()].set_handler_fn(mouse_interrupt_handler);
        idt[InterruptIndex::PrimaryAta.as_usize()].set_handler_fn(primary_ata_interrupt_handler);
        unsafe {
            idt[crate::syscall::SYSCALL_INTERRUPT_ID as usize]
                .set_handler_addr(x86_64::VirtAddr::new(
                    crate::syscall::syscall_handler_asm as *const () as u64,
                ))
                .set_privilege_level(x86_64::PrivilegeLevel::Ring3);
        }
        idt
    };
}

pub fn init_idt() {
    IDT.load();
}

pub fn init_pics() {
    unsafe { PICS.lock().initialize() };
}

extern "x86-interrupt" fn breakpoint_handler(stack_frame: InterruptStackFrame) {
    println!("EXCEPTION: BREAKPOINT\n{:#?}", stack_frame);
}

extern "x86-interrupt" fn double_fault_handler(
    stack_frame: InterruptStackFrame,
    _error_code: u64,
) -> ! {
    panic!("EXCEPTION: DOUBLE FAULT\n{:#?}", stack_frame);
}

/// Sem handler de page fault, qualquer #PF (ex: escrever numa pagina
/// read-only, acessar pagina nao mapeada) vira double fault direto --
/// foi assim que a Fase 13 precisou de disassembly pra achar o bug real
/// (mapear R+X antes de copiar o conteudo do ELF). Imprime CR2 (endereco
/// que causou a falta) + o codigo de erro decodificado antes de travar,
/// pra dar diagnostico direto na tela/serial em vez de só "DOUBLE FAULT".
extern "x86-interrupt" fn page_fault_handler(
    stack_frame: InterruptStackFrame,
    error_code: PageFaultErrorCode,
) {
    panic!(
        "EXCEPTION: PAGE FAULT\nEndereco (CR2): {:?}\nCodigo de erro: {:?}\n{:#?}",
        Cr2::read(),
        error_code,
        stack_frame
    );
}

extern "x86-interrupt" fn keyboard_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;

    let mut port = Port::new(0x60);
    let scancode: u8 = unsafe { port.read() };
    crate::task::keyboard::add_scancode(scancode);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Keyboard.as_u8());
    }
}

extern "x86-interrupt" fn primary_ata_interrupt_handler(_stack_frame: InterruptStackFrame) {
    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::PrimaryAta.as_u8());
    }
}

extern "x86-interrupt" fn mouse_interrupt_handler(_stack_frame: InterruptStackFrame) {
    use x86_64::instructions::port::Port;

    let mut port = Port::new(0x60);
    let data: u8 = unsafe { port.read() };
    crate::mouse::handle_byte(data);

    unsafe {
        PICS.lock()
            .notify_end_of_interrupt(InterruptIndex::Mouse.as_u8());
    }
}
