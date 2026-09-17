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
grid-semantic-title = Might also be relevant
grid-semantic-hint = found by meaning, not exact words
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
editor-mode-write = Write
editor-mode-read = Read
editor-mode-edgeless = Canvas
editor-mode-hint = Switch between writing, reading, and canvas (⌘E)
editor-saved = Saved
editor-saving = Saving…
editor-save-failed = Couldn't save
editor-undo = Undo
editor-redo = Redo
editor-outline = Outline
editor-outline-toggle = Show/hide outline
editor-outline-empty = Add headings (# Heading) to build an outline.
editor-backlinks = Linked from
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
shortcut-toggle-read = Toggle Write/Read
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
