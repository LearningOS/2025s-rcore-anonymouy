//! Process management syscalls
use crate::config::MAX_USER_VA;
use crate::mm::{MapPermission, PTEFlags, VPNRange, VirtAddr};

use crate::task::{insert_framed_area, unmap_framed_area};
use crate::{mm::{PageTable, translated_byte_buffer}, task::{change_program_brk, current_user_token, exit_current_and_run_next, get_syscall_times, suspend_current_and_run_next}, timer::get_time_us};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let user_token = current_user_token();
    // 由于指令对齐，因此我们不必担心usize数据被两页截断。
    let v = translated_byte_buffer(user_token, _ts as *const u8, core::mem::size_of::<TimeVal>());
    let time = get_time_us();
    let sec = time / 1_000_000;
    let usec = time % 1_000_000;
    // 2. 将两个 usize 转化为底层的字节数组 (使用本地字节序 native endian)
    let mut time_bytes = [0u8; 16]; // RV64 下两个 usize 正好是 16 字节
    time_bytes[0..8].copy_from_slice(&sec.to_ne_bytes());
    time_bytes[8..16].copy_from_slice(&usec.to_ne_bytes());

    // 3. 像倒水一样，把这 16 个字节依次倒进 v 提供的一个或两个物理切片中
    let mut offset = 0;
    for slice in v {
        let len = slice.len();
        // 从 time_bytes 中截取对应长度的数据，复制到物理页切片中
        slice.copy_from_slice(&time_bytes[offset .. offset + len]);
        offset += len; // 移动水位线
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");
    let user_token = current_user_token();
    // check read and write operation
    if _trace_request != 2 {
        //invalid user va
        if _id > MAX_USER_VA {
            return -1;
        }
        let user_page_table = PageTable::from_token(user_token);
        let va: VirtAddr = _id.into();
        // permissions not match
        match user_page_table.translate(va.floor()){
            Some(pte) => {
                if !((pte.readable() && _trace_request == 0 ||
                    pte.writable() && _trace_request == 1)
                    && pte.flags() & PTEFlags::U != PTEFlags::empty()) {
                    return -1;
                }
            },
            None => return -1
        };
    }
    let mut v = translated_byte_buffer(user_token, _id as *const u8, core::mem::size_of::<u8>());
    // println!(" v[0][0] is {}", v[0][0]);
    match _trace_request {
        0 => {
            v[0][0] as isize
        }
        1 => {
            v[0][0] = _data as u8;
            0
        },
        2 => {
            get_syscall_times(_id)
        },
        _ => panic!("invalid trace request")
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _prot: usize) -> isize {
    trace!("kernel: sys_mmap implemented!");
    // check validity
    if _start & (1 << 12 - 1) != 0 || _prot > 7 || _prot == 0 {
        return -1;
    }
    let user_token = current_user_token();
    let page_table = PageTable::from_token(user_token);
    let mut map_permissions = MapPermission::U;
    if _prot & 1 != 0 {
        map_permissions |= MapPermission::R;
    }
    if _prot & 2 != 0 {
        map_permissions |= MapPermission::W;
    }
    if _prot & 4 != 0 {
        map_permissions |= MapPermission::X;
    }
    // let map_area = MapArea::new(_start.into(), (_start + _len).into(), MapType::Framed, map_permissions);
    let start: VirtAddr = _start.into();
    // not aligned
    if start.page_offset() != 0 {
        return -1;
    }
    let end: VirtAddr = (_start + _len).into();
    let vpn_range = VPNRange::new(start.floor(), end.ceil());
    for vpn in vpn_range {
        if let Some(pte) = page_table.translate(vpn) {
            if pte.is_valid(){
                return -1;
            }
        }
    }
    insert_framed_area(start, end, map_permissions);
    0
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap IMPLEMENTED");
    if _start & (1 << 12 - 1) != 0 {
        return -1;
    }
    let user_token = current_user_token();
    let mut page_table = PageTable::from_token(user_token);
    let start_va: VirtAddr = _start.into();
    let end_va: VirtAddr = (_start + _len).into();
    // not aligned
    if start_va.page_offset() != 0 {
        return -1;
    }
    if unmap_framed_area(start_va, end_va, &mut page_table) {
        0
    } else {
        -1
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
