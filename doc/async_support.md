# The asynchronous supports

## Introduction

1. The tasks Arceos used are `Thread` both in kernel and user apps.
2. User app tasks use the services of kernel through function call.
3. The execution flow changes through the `yield`, `block` and `sleep` function.

## The asynchronous IO supports in Arceos

There is no asynchronous IO supports in Arceos due to no external interrupts.

Arceos will change the state of task which will wait for an IO from running to ready, and yield the CPU for the next ready task. When this task is scheduled next time, the task will check the wait IO. If the IO is not ready, it will still turn to ready and yield the CPU until the IO is ready.

For example, the smoltcp network stack implementation in the Arceos, when a task is receiving data from the socket which has no data, it will return `Err(AxError::WouldBlock)`, then the current task will yield the CPU just as the description above.

## How to add asynchronous IO supports in Arceos

### Without using Rust Future

For asynchronous IO supports without using Rust Future, the modification will be simple. Still using the smoltcp network stack example, what we need to do is changing the state of current task into `blocked` and put it in the `ATSINTC` interrupt controller, instead of changing it into `ready` and put it into ready queue.

When the interrupt happens, `ATSINTC` will put the blocked task into the ready queue.

### Using Rust Future

The problems must be solved:

1. The task in user apps is `Thread`, we must wrap it into a Future.
2. The execution flow changes directly between the current task and the next ready task without going through the upper level scheduler, which is conflict with the design of Rust Future.
3. The user apps use the services of kernel directly through function call without syscall implementation layer. The execution flow in user apps has no unified entrance which results in wrapping a thread in a coroutine is difficult.

If we have a syscall implementation layer, we can wrap an unwrap the thread in this layer.