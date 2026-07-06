#![no_std]
#![no_main]
#![feature(abi_x86_interrupt)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec::Vec;
use bootloader::{entry_point, BootInfo};
use core::panic::PanicInfo;
use x86_64::VirtAddr;

mod allocator;
mod desktop;
mod framebuffer;
mod fs;
mod gdt;
mod interrupts;
mod memory;
mod mouse;
mod serial;
mod syscall;
mod task;
mod usermode;
mod vga_buffer;

entry_point!(kernel_main);

fn kernel_main(boot_info: &'static BootInfo) -> ! {
    println!("meu-os booted!");
    println!("VGA writer online, println! funcionando.");

    gdt::init();
    interrupts::init_idt();
    interrupts::init_pics();
    mouse::init();
    println!("GDT + IDT + PIC carregados.");

    x86_64::instructions::interrupts::int3();
    println!("Voltou do breakpoint sem crashar.");

    x86_64::instructions::interrupts::enable();
    println!("Interrupcoes de hardware ativas.");

    let phys_mem_offset = VirtAddr::new(boot_info.physical_memory_offset);
    let mut mapper = unsafe { memory::init(phys_mem_offset) };
    let mut frame_allocator = unsafe { memory::BootInfoFrameAllocator::init(&boot_info.memory_map) };

    allocator::init_heap(&mut mapper, &mut frame_allocator).expect("falha ao iniciar heap");
    println!("Heap inicializado.");

    let heap_value = Box::new(41);
    println!("heap_value em {:p}", heap_value);

    let mut vec = Vec::new();
    for i in 0..500 {
        vec.push(i);
    }
    println!("vec em {:p}, len={}", vec.as_slice(), vec.len());

    test_syscall();
    test_fs();
    draw_desktop(boot_info.physical_memory_offset);

    println!("Saltando para ring3 (userland real)...");
    serial_println!("CHECKPOINT: prestes a executar iretq para ring3");
    usermode::jump_to_usermode(&mut mapper);
}

fn test_syscall() {
    let msg = "syscall SYS_WRITE funcionando!\n";
    let mut rax = syscall::SYS_WRITE;
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inout("rax") rax,
            in("rdi") msg.as_ptr() as u64,
            in("rsi") msg.len() as u64,
        );
    }
    println!("syscall SYS_WRITE retornou {} bytes", rax);

    unsafe {
        core::arch::asm!(
            "int 0x80",
            in("rax") syscall::SYS_EXIT,
            in("rdi") 0u64,
        );
    }
}

fn draw_desktop(physical_memory_offset: u64) {
    desktop::setup_palette();
    desktop::init(framebuffer::Framebuffer::new(physical_memory_offset));
    println!("Tela de trabalho desenhada (VGA modo 13h, 320x200).");
}

fn test_fs() {
    let mut filesystem = fs::FS.lock();
    filesystem.create("hello.txt", b"Ola do meu-os!\n");
    filesystem.create("readme.md", b"# meu-os\nSO em Rust.\n");

    println!("arquivos: {:?}", filesystem.list());

    if let Some(data) = filesystem.read("hello.txt") {
        let content = core::str::from_utf8(&data).unwrap_or("<invalido>");
        println!("hello.txt: {}", content);
    }

    filesystem.write("hello.txt", b"conteudo atualizado\n");
    if let Some(data) = filesystem.read("hello.txt") {
        let content = core::str::from_utf8(&data).unwrap_or("<invalido>");
        println!("hello.txt (apos write): {}", content);
    }

    filesystem.delete("readme.md");
    println!("arquivos apos delete: {:?}", filesystem.list());
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    println!("{}", info);
    serial_println!("PANIC: {}", info);
    loop {}
}
