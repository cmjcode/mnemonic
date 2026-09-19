## MNEMONIC — English
## Every key here MUST also exist in id-ID/main.ftl (checked by tests).

## ── Welcome & vault ─────────────────────────────────────────────────
welcome-title = Welcome to MNEMONIC
welcome-subtitle = Your notes, canvases, and PDFs — with an AI assistant that runs on your own computer.
welcome-create-vault = Create New Vault
welcome-open-folder = Open Folder…
welcome-default-vault-name = My Notes
welcome-feature-notes = Write Markdown notes, sketch on canvases, and annotate PDFs
welcome-feature-ai = Search by meaning and ask the AI assistant about your vault
welcome-feature-private = Everything is stored as plain files on your computer
welcome-note-title = Welcome 👋
welcome-note-body =
    # Start here

    This vault is just a regular folder on your computer. Every note is a Markdown file, so your data always stays yours.

    ## The basics
    - [ ] Create a note with ⌘N or the "New Note" button on the left
    - [ ] Type / at the start of a line to insert headings, checklists, tables, and more
    - [ ] Link notes by typing [[ and picking a title
    - [ ] Search anything with ⌘F — results with a similar meaning show up too
    - [ ] Open the command palette with ⌘K to jump anywhere

    ## Tips
    - Notes save automatically when you stop typing.
    - Deleted something by mistake? Click "Undo" on the notification, or restore it from Trash.
    - Right-click a note or folder to see every action.
    - Press ⌘/ to see all keyboard shortcuts.

    Feel free to delete this note at any time.
vault-pick-folder = Choose Vault Folder

## ── Top bar & settings ──────────────────────────────────────────────
topbar-show-sidebar = Show sidebar
topbar-hide-sidebar = Hide sidebar
topbar-ai-assistant = AI assistant
topbar-settings = Settings
topbar-indexing = Indexing { $count }…
topbar-indexing-hint = Preparing meaning search & the AI assistant. You can keep working as usual.
settings-appearance = Appearance
settings-theme-light = Light
settings-theme-dark = Dark
settings-language = Language
settings-switch-vault = Open another vault…
settings-command-palette = Command palette
settings-shortcuts = Keyboard shortcuts

## ── Sidebar ─────────────────────────────────────────────────────────
sidebar-switch-vault = Switch vault
sidebar-recent-vaults = Recent vaults
sidebar-open-other-vault = Open another folder…
sidebar-rescan = Reload files
sidebar-new-other = Create something else…
sidebar-new-canvas = New canvas
sidebar-new-folder = New folder
sidebar-library = Library
sidebar-all = All Documents
sidebar-notes-only = Notes
sidebar-whiteboards-only = Canvases
sidebar-pdfs-only = PDFs
sidebar-archived = Archive
sidebar-trash = Trash
sidebar-folders = Folders
sidebar-folders-empty = No files yet. Create your first note above.
sidebar-folder-empty = Empty folder
sidebar-expand-all = Expand all folders
sidebar-collapse-all = Collapse all folders
sidebar-more-actions = More actions
sidebar-new-note-here = New note here
sidebar-new-canvas-here = New canvas here
sidebar-new-subfolder = New subfolder
sidebar-open = Open
sidebar-move-to = Move to…
sidebar-tags = Labels
sidebar-tags-empty = Add tags in a note's frontmatter to group notes together.
sidebar-manage-tags = Manage labels

## ── Home (grid) ─────────────────────────────────────────────────────
grid-item-count =
    { $count ->
        [one] { $count } item
       *[other] { $count } items
    }
grid-sort = Sort
sort-modified = Last modified
sort-created = Date created
sort-title = Title (A–Z)
sort-color = Color
grid-search-results =
    { $count ->
        [one] { $count } result for “{ $query }”
       *[other] { $count } results for “{ $query }”
    }
grid-clear-search = Clear search
grid-show-all = Show all documents
grid-empty-trash = Empty Trash
grid-trash-info = Items in Trash are permanently deleted automatically after 30 days.
grid-semantic-title = AI search results
grid-semantic-hint = meaning and keywords across note & PDF contents
grid-semantic-match = { $percent }% match

selection-mode-on = Select multiple
selection-mode-off = Done selecting
selection-count = { $count } selected
selection-select-all = Select all
selection-archive = Archive
selection-trash = Move to Trash

notes-new = New Note
notes-pin = Pin to top
notes-unpin = Unpin
notes-pinned = Pinned
card-archive = Archive
card-unarchive = Unarchive
card-trash = Move to Trash
card-restore = Restore
card-delete-permanent = Delete forever
card-color = Color
card-color-none = No color

time-just-now = just now
time-minutes-ago =
    { $count ->
        [one] { $count } minute ago
       *[other] { $count } minutes ago
    }
time-hours-ago =
    { $count ->
        [one] { $count } hour ago
       *[other] { $count } hours ago
    }
time-days-ago =
    { $count ->
        [one] { $count } day ago
       *[other] { $count } days ago
    }

empty-vault-title = Start your first note
empty-vault-body = Write down ideas, make a to-do list, or sketch on a canvas. Everything saves automatically.
empty-vault-tip = Tip: press ⌘N anytime to create a new note.
empty-search-title = No results for “{ $query }”
empty-search-body = Try a more general word, or check the spelling.
empty-trash-title = Trash is empty
empty-trash-body = Items you delete show up here and can be restored for 30 days.
empty-archive-title = Nothing archived yet
empty-archive-body = Archive notes you're done with to keep your home view tidy.
empty-pdf-title = No PDFs yet
empty-pdf-body = Import PDFs to read, annotate, and search them alongside your notes.
empty-filter-title = Nothing here
empty-filter-body = No documents match this filter yet.

## ── Editor ──────────────────────────────────────────────────────────
editor-back = Back
editor-untitled = Untitled
editor-rename-hint = Click to rename
editor-mode-note = Note
editor-mode-edgeless = Canvas
editor-mode-hint = Switch between note and canvas
editor-saved = Saved
editor-saving = Saving…
editor-save-failed = Couldn't save
editor-undo = Undo
editor-redo = Redo
editor-outline = Outline
editor-outline-toggle = Show/hide outline
editor-outline-empty = Add headings (# Heading) to build an outline.
editor-backlinks = Linked from
editor-linked-mentions = Linked mentions
editor-unlinked-mentions-empty = No unlinked mentions.
editor-outgoing-links = Outgoing links
editor-outgoing-empty = No [[links]] in this note yet.
editor-local-graph-empty = No links yet; the graph appears once notes connect.
editor-properties = Properties
editor-tags = Tags
editor-aliases = Aliases
editor-add-tag = + tag
editor-add-alias = + alias
editor-remove = Remove
editor-created = Created
editor-modified = Modified
editor-status-backlinks = { $count } backlinks
editor-char-count = { $count } characters
editor-backlinks-empty = No other notes link here yet.
editor-word-count =
    { $count ->
        [one] { $count } word
       *[other] { $count } words
    }
editor-reading-time = ~{ $minutes } min read
editor-placeholder = Start writing… Type / to insert elements, [[ to link a note.
editor-slash-header = Insert
editor-link-header = Link to note
editor-popup-hint = ↑↓ choose · Enter insert · Esc close

slash-heading-1 = Heading
slash-heading-2 = Subheading
slash-checklist = Checklist
slash-bullet-list = Bulleted list
slash-quote = Quote
slash-code-block = Code block
slash-callout-note = Note callout
slash-callout-warning = Warning callout
slash-table = Table
slash-divider = Divider

## ── Canvas ──────────────────────────────────────────────────────────
canvas-untitled = Untitled Canvas
canvas-empty-hint = Pick a tool on the left, then click or drag to start drawing
canvas-new-sticky = New note
canvas-edit-hint = Esc to finish
canvas-edit-hint-bound = Bound to the note · this text is edited in the Markdown too
canvas-bind = Bind to note
canvas-unbind = Unbind from note
canvas-import-bound = { $count } texts added to the Markdown
canvas-edit-done = Done
canvas-tool-select = Select & move
canvas-tool-pan = Pan canvas
canvas-tool-sticky = Sticky note
canvas-tool-rectangle = Rectangle
canvas-tool-rounded = Rounded rectangle
canvas-tool-ellipse = Ellipse
canvas-tool-diamond = Diamond
canvas-tool-connector = Connector arrow
canvas-tool-pen = Pen
canvas-tool-eraser = Eraser
canvas-import-drawio = Import from Draw.io
canvas-export-drawio = Export to Draw.io
canvas-import-success =
    { $count ->
        [one] Imported { $count } element
       *[other] Imported { $count } elements
    }
canvas-import-skipped =
    { $count ->
        [one] { $count } item couldn't be shown
       *[other] { $count } items couldn't be shown
    }
canvas-import-empty = No diagram content found in this file
canvas-import-failed = Couldn't import the diagram
canvas-export-success = Diagram exported
canvas-export-failed = Couldn't export the diagram
canvas-zoom-in = Zoom in
canvas-zoom-out = Zoom out
canvas-zoom-reset = Reset to 100%
canvas-width-thin = Thin
canvas-width-medium = Medium
canvas-width-thick = Thick
color-yellow = Yellow
color-blue = Blue
color-green = Green
color-pink = Pink
color-purple = Purple
color-orange = Orange
color-red = Red
color-graphite = Graphite

## ── Search, palette, AI assistant ───────────────────────────────────
search-placeholder = Search notes, PDFs, or topics…
command-palette-hint = Type a command or note title…
command-palette-empty = No matches
command-palette-footer = ↑↓ choose · Enter run · Esc close
palette-cat-actions = Actions
palette-cat-navigate = Go to
palette-cat-documents = Documents
palette-cat-view = View
palette-search = Search the vault
palette-ask-ai = Ask the AI assistant
palette-toggle-sidebar = Show/hide sidebar
palette-toggle-theme = Toggle light/dark theme
palette-toggle-language = Ganti ke Bahasa Indonesia

chat-title = AI Assistant
chat-subtitle = Answers from the notes & PDFs in your vault — runs locally.
chat-close = Close
chat-clear = Start a new conversation
chat-empty-title = Ask anything about your vault
chat-empty = Answers come with sources you can open right away.
chat-starter-summary = Summarize the key points from my recent notes
chat-starter-related = Which notes cover the same topic?
chat-starter-ideas = Help me organize ideas from these notes
chat-placeholder = Ask something…
chat-send = Send (Enter)
chat-sources = Sources
chat-open-source = Open this source
chat-thinking = Searching your vault and writing an answer…
chat-error = Couldn't process the question
chat-citation-page = { $name } · p. { $page }

## ── PDF ─────────────────────────────────────────────────────────────
pdf-import = Import PDF
pdf-page-of = Page { $current } of { $total }
pdf-prev-page = Previous page
pdf-next-page = Next page
pdf-more = Page & document actions
pdf-section-page = This page
pdf-section-document = Document
pdf-rotate-left = Rotate left
pdf-rotate-right = Rotate right
pdf-delete-page = Delete page
pdf-delete-page-confirm = This page will be removed and the result saved as a new PDF file. The original file is not changed.
pdf-split = Extract pages
pdf-split-to = to
pdf-split-go = Save as new PDF…
pdf-merge = Merge with another PDF…
pdf-op-success = Saved as a new file
pdf-op-error = process the PDF
pdf-render-unavailable = This PDF page can't be displayed
pdf-annotate-none = Browse
pdf-annotate-highlight = Highlight
pdf-annotate-underline = Underline
pdf-annotate-sticky = Sticky note
pdf-annotate-text = Insert text
pdf-annotate-color = Annotation color
pdf-annotate-sticky-prompt = Sticky note text
pdf-annotate-text-prompt = Text to insert
pdf-annotate-add = Add
pdf-annotate-cancel = Cancel
pdf-unsaved-annotations =
    { $count ->
        [one] { $count } unsaved annotation
       *[other] { $count } unsaved annotations
    }
pdf-metadata-button = Document info
pdf-metadata-window-title = Document info
pdf-metadata-field-title = Title
pdf-metadata-field-author = Author
pdf-metadata-field-keywords = Keywords
pdf-metadata-hint = Changes are applied when you press Save or Export.
pdf-metadata-close = Done
pdf-save = Save
pdf-save-confirm = Annotations and document info will be saved into the original file. A backup (.bak) is created automatically.
pdf-save-success = Saved. Backup: { $backup }
pdf-export = Export as new file…

## ── Dialogs ─────────────────────────────────────────────────────────
confirm-cancel = Cancel
confirm-yes = Delete forever
confirm-delete-title = Delete forever?
confirm-delete-body = This note will be deleted permanently and can't be recovered.
conflict-title = File changed outside the app
conflict-body = “{ $name }” was modified by another program since it was opened here, and you have unsaved changes. Choose which version to keep.
conflict-reload = Reload from disk
conflict-overwrite = Overwrite with this version
conflict-copy = Save as a copy
confirm-empty-trash-title = Empty Trash?
confirm-empty-trash-body = Every note in Trash will be deleted permanently. This can't be undone.
confirm-empty-trash-yes = Empty Trash
folder-new-title = New folder
folder-new-message = Give the new folder a name.
folder-new-placeholder = e.g. Projects, Classes, Recipes
folder-new-confirm = Create folder
rename-folder-title = Rename folder
rename-file-title = Rename
rename-message = New name for “{ $name }”.
rename-placeholder = New name
rename-confirm = Save
move-modal-title = Move “{ $name }”
move-modal-search = Search folders…
move-modal-root = Vault root folder
tag-manager-title = Manage labels
tag-manager-empty = No labels yet. Add tags in a note's frontmatter, for example: tags: [work, ideas]
tag-note-count =
    { $count ->
        [one] { $count } note
       *[other] { $count } notes
    }
tag-rename = Rename
tag-delete = Delete label

## ── Shortcuts ───────────────────────────────────────────────────────
shortcut-palette = Command palette
shortcut-new-note = New note
shortcut-search = Search
shortcut-save = Save now
shortcut-toggle-source = Toggle full Markdown source
shortcut-print = Print note
shortcut-sidebar = Show/hide sidebar
shortcut-ai = AI assistant
shortcut-back = Back to home
shortcut-slash = Insert element (at line start)
shortcut-wikilink = Link to another note

## ── Notifications ───────────────────────────────────────────────────
toast-undo = Undo
toast-note-trashed = “{ $title }” moved to Trash
toast-item-trashed = “{ $title }” moved to Trash
toast-note-restored = “{ $title }” restored
toast-restored = Restored
toast-archived = Note archived
toast-unarchived = Note unarchived
toast-batch-archived =
    { $count ->
        [one] { $count } note archived
       *[other] { $count } notes archived
    }
toast-deleted-permanently =
    { $count ->
        [one] { $count } note deleted forever
       *[other] { $count } notes deleted forever
    }
toast-moved = Moved to { $folder }
toast-name-taken = “{ $name }” already exists in this folder
toast-open-failed = This file can't be opened — it may have been moved or deleted
toast-pdfs-imported = { $count } PDFs imported

## ── Errors ──────────────────────────────────────────────────────────
error-banner = Couldn't { $context }: { $error }
error-context-autosave = autosave
error-context-indexing = indexing notes for search
error-context-save-note = save the note
error-context-delete-note = delete the file
error-context-move-note = restore the note
error-context-create-note = create a note
error-context-open-vault = open the vault
error-context-create-folder = create the folder
error-context-rename-folder = rename the folder
error-context-move-file = move the file
error-context-delete-folder = move the folder to Trash
error-context-trash-note = move the note to Trash

## ── Tambahan / Additional ──
toast-close-unsaved = Your latest changes couldn't be saved. Close again to quit without saving.
canvas-count-sticky =
    { $count ->
        [one] { $count } sticky note
       *[other] { $count } sticky notes
    }
canvas-count-shapes =
    { $count ->
        [one] { $count } shape
       *[other] { $count } shapes
    }
canvas-count-connectors =
    { $count ->
        [one] { $count } connector
       *[other] { $count } connectors
    }
canvas-count-strokes =
    { $count ->
        [one] { $count } stroke
       *[other] { $count } strokes
    }
canvas-count-empty = Empty canvas

## Relationship graph & links
graph-title = Relationship graph
graph-empty = No notes to show yet.
graph-filter = Filter nodes…
graph-show-orphans = Show notes without links
graph-show-ghosts = Show links to notes that don't exist yet
graph-show-pdfs = Show PDFs
graph-show-semantic = AI connections (similar meaning)
graph-show-semantic-hint = Dashed lines connect documents with similar content that aren't linked yet.
graph-counts = { $nodes } nodes · { $edges } connections
graph-fit = Fit to screen
graph-kind-note = Note
graph-kind-canvas = Canvas
graph-kind-pdf = PDF
graph-kind-ghost = Not created yet
graph-node-links = { $count ->
        [one] { $count } connection
       *[other] { $count } connections
    }
shortcut-graph = Relationship graph
shortcut-daily = Today's daily note
shortcut-cheatsheet = This cheat sheet
editor-unlinked-mentions = Unlinked mentions
editor-link-mention = Turn into link
editor-related = Related (AI)
editor-related-empty = No similar documents yet.
editor-insert-link = Link from this note
editor-local-graph = Local graph
toast-links-updated = Updated links in { $count ->
        [one] { $count } note
       *[other] { $count } notes
    }
toast-mention-linked = Added a link in "{ $title }"
toast-link-pdf-missing = PDF "{ $name }" isn't in the vault
toast-rerank-on = AI reranking on (the model downloads on the next search)
toast-rerank-off = AI reranking off
toast-note-reloaded = “{ $title }” reloaded from disk
toast-conflict-copy-saved = This version was saved as “{ $title }”
toast-filenames-migrated = { $count } files renamed after their titles
palette-migrate-filenames = Rename UUID files after their titles
palette-daily-note = Open today's daily note
palette-insert-template = Insert template: { $name }
palette-cat-templates = Templates
palette-rerank-on = Turn on AI reranking for search
palette-rerank-off = Turn off AI reranking for search
grid-match-keyword = keyword
grid-match-both = { $percent }% · keyword

## Sheets — CSV / XLSX (§3.8)
sidebar-new-sheet = New sheet
sidebar-new-sheet-here = New sheet here
sheet-import = Import spreadsheet
sheet-import-filter = Spreadsheets
sheet-untitled = Untitled Sheet
sheet-read-only = Read-only
sheet-read-only-workbook = Workbooks open read-only so their formulas, formatting and charts are never lost. Use “Convert to CSV” to edit a copy.
sheet-read-only-encoding = This file isn't UTF-8, so it opens read-only — saving could corrupt its characters.
sheet-filter = Filter rows…
sheet-add-row = Add row
sheet-add-column = Add column
sheet-export-xlsx = Export as XLSX
sheet-convert-csv = Convert to CSV (editable copy)
sheet-size = { $rows } rows × { $cols } columns
sheet-showing = { $shown } shown
sheet-unsaved = unsaved
sheet-empty = This sheet has no columns yet.
sheet-sort-hint = Click to sort · right-click for more
sheet-sort-asc = Sort ascending
sheet-sort-desc = Sort descending
sheet-rename-column = Rename column
sheet-insert-column-left = Insert column left
sheet-insert-column-right = Insert column right
sheet-delete-column = Delete column
sheet-insert-row-above = Insert row above
sheet-insert-row-below = Insert row below
sheet-delete-row = Delete row
sheet-stats = { $col } — count { $count } · sum { $sum } · avg { $avg } · min { $min } · max { $max }
sheet-stats-text = { $col } — { $filled } filled
chat-citation-row = { $name } · row { $row }
toast-sheet-exported = Exported { $name }
toast-sheet-converted = Created { $name }
toast-sheet-conflict = The file changed on disk, so your edits were saved to { $name } instead
error-context-open-sheet = open the sheet
error-context-save-sheet = save the sheet
error-context-export-sheet = export the sheet
error-context-import-sheet = import the spreadsheet
toast-link-sheet-missing = Sheet "{ $name }" isn't in the vault
graph-kind-sheet = Sheet

## Live editor & reading themes (§3.2.1, §3.2.5)
editor-live-hint = Write Markdown… / to insert, [[ to link, Esc when done
reading-theme = Reading theme
reading-theme-open-folder = Open theme plugin folder
reading-theme-reload = Reload themes
reading-theme-reloaded = { $count } themes loaded
reading-theme-note-override = This note uses “{ $name }” (frontmatter theme:)
theme-load-problems = { $count } problems loading themes — see the log
theme-folder-failed = Could not open the theme folder
export-menu = Print & export
export-print = Print…
export-pdf = Export PDF…
export-html = Export HTML…
export-print-opened = Print page opened in the browser — theme colours print too
export-pdf-started = Creating PDF…
export-done = Saved: { $file }
export-failed = Export failed
export-no-browser = PDF needs Chrome, Edge, Chromium or Brave. Use Print → Save as PDF, or export HTML.
palette-print = Print this note
palette-export-pdf = Export this note to PDF
palette-export-html = Export this note to HTML
palette-toggle-source = Toggle full Markdown source
palette-reading-theme = Reading theme: { $name }
palette-open-theme-folder = Open theme plugin folder
