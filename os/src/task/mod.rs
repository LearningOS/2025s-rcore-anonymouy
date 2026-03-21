//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::mm::{MapPermission, PageTable, VirtAddr};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::{__switch,add_switch_time, get_switch_time, switch_refresh_and_return};
pub use task::{TaskControlBlock, TaskStatus};
use crate::config::MAX_SYSCALL_NUM;
use crate::timer::{get_time_ms};

pub use context::TaskContext;
static mut TMP_TIME: usize = 0;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
       // timing starts
        refresh_and_return();
        switch_refresh_and_return();
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
        // add kernel time to current app
        inner.tasks[cur].kernel_time += refresh_and_return();
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
        // add kernel time to current app
        inner.tasks[cur].kernel_time += refresh_and_return();
        println!("user time {}ms, kernel time {}ms", inner.tasks[cur].user_time, inner.tasks[cur].kernel_time);
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// insert framed area
    fn insert_framed_area(&self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission) {
            let mut inner = self.inner.exclusive_access();
            let current_task = inner.current_task;
            inner.tasks[current_task].insert_framed_area(start_va, end_va, permission);
    }

    /// unmap framed area
    fn unmap_framed_area(&self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        page_table: &mut PageTable) -> bool {
            let mut inner = self.inner.exclusive_access();
            let current_task = inner.current_task;
            inner.tasks[current_task].unmap_framed_area(start_va, end_va, page_table)
        }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            switch_refresh_and_return();
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            add_switch_time(switch_refresh_and_return());
            // go back to user mode
        } else {
            println!("Switch time {}us in total", get_switch_time());
            panic!("All applications completed!");

        }
    }

    /// add current user time
    pub fn add_current_user_time(&self, time: usize) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].user_time += time;
    } 
    /// add current kernel time
    pub fn add_current_kernel_time(&self, time: usize) {
        let mut inner = self.inner.exclusive_access();
        let current = inner.current_task;
        inner.tasks[current].kernel_time += time;
    }

    /// increase number of syscall
    pub fn add_up_syscall(&self, syscall_id: usize) {
        let mut inner = self.inner.exclusive_access();
        let current_task = inner.current_task;
        let current_calls = &mut inner.tasks[current_task].calls;
        current_calls.entry(syscall_id)
            .and_modify(|count| *count += 1)
            .or_insert(1);
    }
    /// get syscall times
    pub fn get_syscall_times(&self, syscall_id: usize) -> isize {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        if syscall_id >= MAX_SYSCALL_NUM {
            -1
        } else {
            *inner
                .tasks[current]
                .calls
                .get(&syscall_id)
                .unwrap_or(&0)
                as isize
        }
    }
    
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}

/// refresh time and return gap time
pub fn refresh_and_return() -> usize {
    // let present_time = get_time_ms();
    // let gap = present_time - unsafe { TMP_TIME };
    // unsafe { TMP_TIME = present_time };
    // gap
    let time_before = unsafe { TMP_TIME };
    unsafe { TMP_TIME = get_time_ms(); 
    TMP_TIME - time_before
    }
}

/// pub fn add current user time
pub fn add_current_user_time() {
    TASK_MANAGER.add_current_user_time(refresh_and_return());
}

/// pub fn add current kernel time
pub fn add_current_kernel_time() {
    TASK_MANAGER.add_current_kernel_time(refresh_and_return());
}

/// increase numbers of syscall timing
pub fn add_up_syscall(syscall_id: usize) {
    TASK_MANAGER.add_up_syscall(syscall_id);
}

/// get times of syscall
pub  fn get_syscall_times(syscall_id: usize) -> isize {
    TASK_MANAGER.get_syscall_times(syscall_id)
}

/// Insert framed area
pub fn insert_framed_area(start_va: VirtAddr,
        end_va: VirtAddr,
        permission: MapPermission) {
    TASK_MANAGER.insert_framed_area(start_va, end_va, permission);
}

/// Unmap framed area
/// Consider unmap areas which is not distributed by the app itself
pub fn unmap_framed_area(start_va: VirtAddr,
        end_va: VirtAddr,
        page_table: &mut PageTable) -> bool {
    TASK_MANAGER.unmap_framed_area(start_va, end_va, page_table)
}