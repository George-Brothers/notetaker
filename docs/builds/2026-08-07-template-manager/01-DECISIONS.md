# Decisions

- Templates live in the existing persisted settings file and are returned through the current settings IPC.
- Built-in templates are editable so Mr. Brothers can adjust their format, but General notes is never deletable.
- A recording whose saved template no longer exists falls back to General notes when it is processed.
- Template edits apply to subsequent processing and reprocessing; existing summary files are not rewritten.
