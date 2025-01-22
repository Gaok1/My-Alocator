use std::{
    alloc::GlobalAlloc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

#[derive(Clone, Copy)]
pub struct Slot {
    pub size: usize,
    pub index: usize,
}

pub struct AlphaAlocator {
    times_called: AtomicUsize,
    memory: [u8; 3072],
    size: usize,
    free: AtomicUsize,
    used_slots: Mutex<[Slot; 3072]>,
    next_offset: AtomicUsize,
    historic : Mutex<[Option<u32>; 3072]>, //armazena pedidos de alocação
}

impl AlphaAlocator {
    fn alocate_slot(&self, index: usize, size: usize) -> Option<Slot> {
        let mut guard = self.used_slots.lock().unwrap();
        if let Some(slot) = guard.iter_mut().find(|s| s.size == 0) {
            slot.size = size;
            slot.index = index;
            Some(*slot)
        } else {
            None
        }
    }

    pub fn identify_adress(&self, ptr: *mut u8) -> Option<usize> {
        let adress = &self.memory[0] as *const u8 as *mut u8;
        let offset = ptr as usize - adress as usize;
        if offset < self.size {
            Some(offset) //index do slot
        } else {
            None
        }
    }

    pub fn print_historic(&self) {
        println!("\n\n Historic of allocations\n\n");
        let guard = self.historic.lock().unwrap();
        for i in 0..3072 {
            if let Some(value) = guard[i] {
                println!("Slot {} foi alocado para {} bytes", i, value);
            }
        }
    }

    pub fn reg_historic(&self, size: usize) {
        for i in 0..self.size {
            let mut guard = self.historic.lock().unwrap();
            if guard[i].is_none() {
                guard[i] = Some(size as u32);
                break;
            }
        }
    }

}

unsafe impl GlobalAlloc for AlphaAlocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        self.reg_historic(layout.size());
        self.times_called.fetch_add(1, Ordering::SeqCst);
        let size = layout.size();
        if size > self.free.load(Ordering::Relaxed) {
            self.print_historic();
            panic!("Out of memory");
        }
        let old_offset = self.next_offset.fetch_add(size, Ordering::SeqCst);
        if old_offset + size > self.size {
            self.next_offset.fetch_sub(size, Ordering::SeqCst);
            self.print_historic();
            panic!("Out of memory");
        }
        self.free.fetch_sub(size, Ordering::SeqCst);
        if let Some(_slot) = self.alocate_slot(old_offset, size) {
            let ptr = self.memory.as_ptr().add(old_offset) as *mut u8;
            ptr
        } else {
            self.next_offset.fetch_sub(size, Ordering::SeqCst);
            self.free.fetch_add(size, Ordering::SeqCst);
            std::ptr::null_mut()
        }
    }
  
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        let dealocated_index = self.identify_adress(ptr).unwrap();  
        let mut guard = self.used_slots.lock().unwrap();
        let slot = &mut guard[dealocated_index];
        slot.index = 0;
        slot.size = 0;
    }
}

#[global_allocator]
static A: AlphaAlocator = AlphaAlocator {
    times_called: AtomicUsize::new(0),
    memory: [0; 3072],
    size: 3072,
    free: AtomicUsize::new(3072),
    used_slots: Mutex::new([Slot { size: 0, index: 0 }; 3072]),
    next_offset: AtomicUsize::new(0),
    historic: Mutex::new([None; 3072]),
};

fn main() {
    println!("Hello, world!");
    let pointer = Box::new(10);
    //println!("Box pointer = {:?}", pointer);
    // let mut v = Vec::new();
    // for i in 0..400 {
    //     v.push(i);
    // }
    //println!("v = {:?}", v);
    println!("Done");
    println!(
        "Alocador foi chamado {} vezes",
        A.times_called.load(Ordering::SeqCst)
    );
    A.print_historic();
}
