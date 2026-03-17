//! Process management syscalls
use core::ptr::write_volatile;

use alloc::vec::Vec;

use crate::{mm::{VirtAddr, translated_byte_buffer}, task::{change_program_brk, current_user_token, exit_current_and_run_next, suspend_current_and_run_next}, timer::get_time_us};

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
    match _trace_request {
        0 => unsafe {
            *(_id as *const isize)
        }
        1 => {
            unsafe { (_id as *mut u8).write_volatile(_data as u8) }; 
            0
        },
        2 => {
            get_syscall_times(_id)
        },
        _ => panic!("invalid trace request")
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!("kernel: sys_mmap NOT IMPLEMENTED YET!");
    -1
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    -1
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
