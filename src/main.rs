use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::alloc::{GlobalAlloc, Layout};

const MEMORY_SIZE: usize = 30000;
const SLOT_SIZE: usize = 100; //Maximo de ponteiros
const HISTORIC_SIZE: usize = 400; 

#[derive(Clone, Copy)]
pub struct Slot {
    // `index` = offset (onde começa)
    // `size` = tamanho do bloco
    pub size: usize,
    pub index: usize,
}

pub struct AlphaAlocator {
    times_called: AtomicUsize,
    memory: [u8; MEMORY_SIZE], // memória
    free: AtomicUsize,
    used_slots: Mutex<[Slot; SLOT_SIZE]>, 
    historic: Mutex<[Option<u32>; HISTORIC_SIZE]>,
}

impl AlphaAlocator {
    pub const fn new() -> Self {
        AlphaAlocator {
            times_called: AtomicUsize::new(0),
            memory: [0; MEMORY_SIZE],
            free: AtomicUsize::new(MEMORY_SIZE),
            used_slots: Mutex::new([Slot { size: 0, index: 0 }; SLOT_SIZE]),
            historic: Mutex::new([None; HISTORIC_SIZE]),
        }
    }

    /// Adiciona no histórico só pra gente ter um rastro do que já foi pedido
    fn reg_historic(&self, size: usize) {
        let mut guard = self.historic.lock().unwrap();
        for slot in guard.iter_mut() {
            if slot.is_none() {
                *slot = Some(size as u32);
                break;
            }
        }
    }

    /// Printa o histórico de alocações, caso quisermos depurar
    pub fn print_historic(&self) {
        println!("\n\nHistoric of allocations\n\n");
        let guard = self.historic.lock().unwrap();
        for (i, maybe_value) in guard.iter().enumerate() {
            if let Some(value) = maybe_value {
                println!("Slot {} foi alocado para {} bytes", i, value);
            }
        }
    }

    /// Identifica o offset (index do Slot) correspondente ao ponteiro
    pub fn identify_adress(&self, ptr: *mut u8) -> Option<usize> {
        let base = self.memory.as_ptr() as usize; // endereço do início
        let alvo = ptr as usize;
        if alvo < base {
            return None;
        }
        let offset = alvo - base;
        if offset < self.memory.len() {
            Some(offset)
        } else {
            None
        }
    }

    /// Retorna o offset onde pode alocar `size` bytes (baseado nos Slots usados).
    /// Se não encontrar espaço, retorna None.
    fn find_free_offset(&self, size: usize) -> Option<usize> {
        let guard = self.used_slots.lock().unwrap();

        // 1) Coletar todos os blocos que estão em uso (size > 0)
        //    e jogar num array local (ou stack) pra gente ordenar
        //    e achar os buracos.
        let mut used_count = 0;
        let mut temp_blocks = [Slot { size: 0, index: 0 }; MEMORY_SIZE];
        
        for slot in guard.iter() {
            if slot.size > 0 {
                temp_blocks[used_count] = *slot;
                used_count += 1;
            }
        }

        // 2) Ordenar esses blocos por offset (index)
        //    Como não podemos usar sort do Vec, faz um bubble sort safado
        //    ou qualquer sort estático. Vou exemplificar um bubble sort aqui:
        for i in 0..used_count {
            for j in 0..(used_count - 1 - i) {
                if temp_blocks[j].index > temp_blocks[j + 1].index {
                    let tmp = temp_blocks[j];
                    temp_blocks[j] = temp_blocks[j + 1];
                    temp_blocks[j + 1] = tmp;
                }
            }
        }

        // 3) Tentar encaixar antes do primeiro bloco
        if used_count == 0 {
            // Nenhum bloco em uso, podemos alocar no offset 0
            if size <= self.memory.len() {
                return Some(0);
            } else {
                return None;
            }
        } else {
            // Se o primeiro bloco começa depois de 0, vamos ver se cabe
            // do offset 0 até o início do primeiro:
            let first_block = temp_blocks[0];
            if first_block.index >= size {
                // cabe antes do primeiro bloco
                return Some(0);
            }
        }

        // 4) Tentar encaixar entre blocos consecutivos
        for i in 0..(used_count - 1) {
            let this_block = temp_blocks[i];
            let next_block = temp_blocks[i + 1];

            let end_this = this_block.index + this_block.size;
            let gap = next_block.index - end_this;
            if gap >= size {
                // Achamos um buraco
                return Some(end_this);
            }
        }

        // 5) Tentar encaixar depois do último bloco
        let last_block = temp_blocks[used_count - 1];
        let end_last = last_block.index + last_block.size;
        let space_after = self.memory.len() - end_last;
        if space_after >= size {
            return Some(end_last);
        }

        // Se não achou buraco, bora mandar user pastar
        None
    }

    /// Salva um novo bloco (offset + size) em `used_slots`
    /// Retorna true se conseguiu, false se não conseguiu achar "Slot livre".
    fn register_slot(&self, offset: usize, size: usize) -> bool {
        let mut guard = self.used_slots.lock().unwrap();
        // Pega o primeiro slot que estiver livre (size=0)
        if let Some(slot) = guard.iter_mut().find(|s| s.size == 0) {
            slot.index = offset;
            slot.size = size;
            true
        } else {
            false
        }
    }
}

unsafe impl GlobalAlloc for AlphaAlocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Registrar histórico
        self.reg_historic(layout.size());
        self.times_called.fetch_add(1, Ordering::SeqCst);

        let size = layout.size();
        if size > self.free.load(Ordering::Relaxed) {
            self.print_historic();
            panic!("Out of memory (sem espaço total)");
        }

        // Acha um offset livre via varredura
        if let Some(offset) = self.find_free_offset(size) {
            // Tenta registrar esse bloco em used_slots
            if self.register_slot(offset, size) {
                // Ajusta o free
                self.free.fetch_sub(size, Ordering::SeqCst);
                
                // Cria o ponteiro de retorno (endereço = base + offset)
                let ptr = self.memory.as_ptr().add(offset) as *mut u8;
                return ptr;
            } else {
                // Nenhum Slot livre no array pra registrar o bloco (muito bizarro, mas pode acontecer)
                self.print_historic();
                panic!("Sem entrada livre em used_slots pra registrar novo bloco!");
            }
        } else {
            // Não achou buraco
            self.print_historic();
            panic!("Out of memory (fragmentação detecteda)!");
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Vamos identificar qual slot corresponde a esse ponteiro:
        if let Some(offset) = self.identify_adress(ptr) {
            let mut guard = self.used_slots.lock().unwrap();
            // Acha o slot que tenha (index == offset) e (size == layout.size())
            //   - poderia checar também se bate o size, se for outro s.size, é estranho
            if let Some(slot) = guard.iter_mut().find(|s| s.index == offset) {
                // Liberar (zera size e index)
                slot.index = 0;
                slot.size = 0;
                // Devolver a memória pro 'free'
                self.free.fetch_add(layout.size(), Ordering::SeqCst);
            } else {
               
                eprintln!("dealloc: não achou slot com offset {}, algo errado!", offset);
            }
        } else {
            eprintln!("dealloc: ponteiro fora da nossa memória, rust pirou!");
        }
    }
}

#[global_allocator]
static ALOCATOR : AlphaAlocator = AlphaAlocator {
    times_called: AtomicUsize::new(0),
    memory: [0; MEMORY_SIZE],
    free: AtomicUsize::new(MEMORY_SIZE),
    used_slots: Mutex::new([Slot { size: 0, index: 0 }; SLOT_SIZE]),
    historic: Mutex::new([None; HISTORIC_SIZE]),
};


fn main(){

}