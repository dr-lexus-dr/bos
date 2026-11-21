#!/bin/sh
# ring's x86-64 backend requires the hardware SSE2 ABI. Keep the original
# soft-float target for interrupt handlers and the userspace context switch.
# Cargo stable cannot yet scope target features to a single package.
compiler=$1
shift
case " $* " in
    *" --crate-name ring "*)
        case " $* " in
            *" --target x86_64-unknown-none "*) set -- "$@" "-Ctarget-feature=-soft-float,+sse,+sse2" ;;
        esac
        ;;
esac
exec "$compiler" "$@"
