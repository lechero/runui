# Runui

Runui helps people find a folder's commands, launch persistent runs, and return to their output.

## Language

**Project**:
A folder whose command definitions belong together. A repository can contain several projects.
_Avoid_: using repository and project interchangeably.

**Workspace**:
A set of projects or packages declared by a tool's workspace configuration. Its membership can extend beyond the folders found by a depth-limited scan.
_Avoid_: using workspace to mean every descendant directory.

**Checkout**:
One working copy of a repository, including a Git worktree. Two checkouts of the same repository can have different definitions and working directories.

**Command definition**:
A named operation declared by a project tool or explicitly saved by a user. It describes what can be invoked, rather than one execution of it.
_Avoid_: calling a command definition a run.

**Generated action**:
An operation runui offers from known tool semantics, such as building a Cargo package, rather than a named operation declared by that project. Its origin must remain distinguishable from a command definition.

**Invocation**:
The concrete command, inputs, working directory, and shell or tool context chosen for execution. Different arguments or directories can make different invocations of one command definition.

**Run**:
One execution managed by a persistent supervisor, with its own identity, output, and lifecycle. A repeated invocation creates another run.
_Avoid_: using task to mean both a command definition and a run.

**Favorite**:
A user's named, saved command and folder, available without rediscovering a project definition. A discovered command may also have a favorite badge.

**Usage history**:
A record of invocations launched through runui, used to find and repeat previous choices. Looking at a result does not constitute another use of its command.
_Avoid_: treating result review or imported shell records as a count of runui launches.

**Shell history entry**:
A command recorded by a particular shell or history tool. Its working directory, time, and outcome are known only when the source actually recorded them.
_Avoid_: assuming a shell history entry is a project command or a supervised run.

**Promotion**:
A deliberate choice to save a private shell history entry as a shared favorite. Reading, searching and copying private history do not promote it.

**Log**:
The place to review finished run results and revisit retained runs and their output. It has an unread view and an all-results view.
_Avoid_: using Log to mean imported shell history or unlimited output retention.

**Review**:
Acknowledging a finished run result after viewing it or explicitly marking it read. Review clears that result's notice without deleting the run or changing its outcome; running processes cannot be reviewed as finished results.

**Preset**:
A named set of input choices for invoking a command definition. A preset is reusable configuration, not an execution result.

**Target**:
A tool-specific concept that needs qualification, such as a Cargo binary target or a Make target. It is not a general synonym for command, project, or process.
