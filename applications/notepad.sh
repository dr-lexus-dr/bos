#!/bin/bash
# Simple notepad for bOS
BOS_ROOT="${BOS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}"
TEXT_FILE="$BOS_ROOT/users/default/documents/notes.txt"
mkdir -p "$(dirname "$TEXT_FILE")" || exit 1

# Prefer the standard bOS text editor when available.
TEXT_EDITOR="$BOS_ROOT/system/text-editor"
if [ -x "$TEXT_EDITOR" ]; then
    exec "$TEXT_EDITOR" "$TEXT_FILE"
fi

echo "bOS Notepad"
echo "==========="

if [ -f "$TEXT_FILE" ]; then
    cat "$TEXT_FILE"
    echo ""
    echo "Press Ctrl+C to save and exit, or wait..."
    cat "$TEXT_FILE"
else
    echo "No notes found. Creating new note..."
    touch "$TEXT_FILE"
fi

# Simple edit loop
while true; do
    read -p "Enter text (or 'save' to save and exit): " user_input
    if [ "$user_input" = "save" ]; then
        break
    elif [ -n "$user_input" ]; then
        echo "$user_input" >> "$TEXT_FILE"
        echo "Added to notes."
    fi
done

echo "Notes saved to $TEXT_FILE"
