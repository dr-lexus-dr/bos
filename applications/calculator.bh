#!/bin/bash
# Simple calculator for bOS
echo "bOS Calculator"
echo "============="
echo "Enter calculation (or 'quit' to exit):"
while read expression; do
    if [ "$expression" = "quit" ]; then
        break
    fi
    # Use bc for calculation if available, otherwise simple arithmetic
    if command -v bc &>/dev/null; then
        result=$(echo "$expression" | bc 2>/dev/null)
        if [ $? -eq 0 ]; then
            echo "Result: $result"
        else
            echo "Error in calculation"
        fi
    else
        # Simple arithmetic using shell
        result=$((expression))
        echo "Result: $result"
    fi
    echo "Enter next calculation (or 'quit' to exit):"
done
