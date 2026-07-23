use std::{io, mem, mem::size_of};

use libc::{CPU_SET, cpu_set_t, sched_setaffinity};
use mem::zeroed;

// pin_current_thread_to_cpu -------------------------------------------

pub(crate) fn pin_current_thread_to_cpu(cpu: usize) -> io::Result<()> {
    let set = single_cpu(cpu);

    unsafe {
        if sched_setaffinity(
            0, // current thread
            size_of::<cpu_set_t>(),
            &set,
        ) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

// cpus_from_ranges ----------------------------------------------------

pub(crate) fn cpus_from_ranges(cpus: &[(usize, usize)]) -> Vec<usize> {
    let mut res = Vec::new();

    for &(start, end) in cpus {
        res.extend(start..=end);
    }

    res.sort_unstable();
    res.dedup();
    res
}

// single_cpu ----------------------------------------------------------

fn single_cpu(cpu: usize) -> cpu_set_t {
    unsafe {
        let mut set = zeroed::<cpu_set_t>();
        CPU_SET(cpu, &mut set);
        set
    }
}
