app-title = LONTAR

vault-empty = No notes yet. Tap the + button to create a new note.
vault-pick-folder = Choose Vault Folder
vault-select-prompt = Choose a Vault folder to start taking notes.

notes-count =
    { $count ->
        [one] { $count } note
       *[other] { $count } notes
    }

notes-new = New Note
notes-delete = Delete
notes-pin = Pin
notes-unpin = Unpin

app-status-ready = Ready

editor-back = Back
editor-mode-source = Source
editor-mode-live-preview = Live Preview
editor-mode-reading = Reading
editor-undo = Undo
editor-redo = Redo
editor-outline = Outline
editor-backlinks = Backlinks
editor-backlinks-empty = No other notes link here yet.
editor-word-count = { $count } words
editor-reading-time = ~{ $minutes } min read

sidebar-all = All Notes
sidebar-archived = Archived
sidebar-trash = Trash
sidebar-tags = Labels
sidebar-manage-tags = Manage Labels

sort-modified = Last Modified
sort-created = Date Created
sort-title = Title
sort-color = Color

selection-mode-on = Select Multiple
selection-mode-off = Cancel Selection
selection-archive = Archive Selected
selection-trash = Trash Selected

card-archive = Archive
card-unarchive = Unarchive
card-trash = Move to Trash
card-restore = Restore
card-delete-permanent = Delete Permanently

confirm-delete-title = Delete Permanently?
confirm-delete-body = This note will be permanently deleted and cannot be recovered.
confirm-yes = Yes, Delete
confirm-cancel = Cancel

tag-manager-title = Manage Labels
tag-rename = Rename
tag-delete = Delete

grid-empty-filtered = No notes match.

nav-notes = Notes
nav-search = Search
nav-chat = Chat
nav-pdf = PDF

search-placeholder = Search notes or documents...
search-button = Search
search-prompt = Type a keyword to search the whole vault (title, content, and semantic meaning).
search-no-results = No matching results found.
search-error = Search failed

chat-placeholder = Ask something about your notes/documents...
chat-send = Send
chat-empty = Start a conversation by asking about your vault's contents.
chat-sources = Sources:
chat-thinking = Thinking...
chat-error = Failed to process the question

pdf-import = Import PDF
pdf-library-empty = No PDFs imported yet. Tap "Import PDF" to add one.
pdf-open = Open
pdf-remove = Remove from List
pdf-back = ← Back
pdf-page-of = Page { $current } / { $total }
pdf-zoom = Zoom
pdf-rotate-left = ↺ Rotate Left
pdf-rotate-right = ↻ Rotate Right
pdf-delete-page = Delete Page
pdf-delete-page-confirm = This page will be removed into a new PDF file. The original file is left unchanged. Continue?
pdf-split = Split pages
pdf-split-to = to
pdf-split-go = Split into New File
pdf-merge = Merge with Another PDF...
pdf-op-success = Saved successfully as a new file
pdf-op-error = PDF operation failed
pdf-render-unavailable = Can't display this page (PDFium library not found)

pdf-annotate-none = None (browse)
pdf-annotate-highlight = Highlight
pdf-annotate-underline = Underline
pdf-annotate-sticky = Sticky Note
pdf-annotate-text = Inject Text
pdf-annotate-color = Color
pdf-annotate-sticky-prompt = Sticky Note Text
pdf-annotate-text-prompt = Text to Inject
pdf-annotate-add = Add
pdf-annotate-cancel = Cancel
pdf-metadata-button = Metadata
pdf-metadata-window-title = Edit PDF Metadata
pdf-metadata-field-title = Title
pdf-metadata-field-author = Author
pdf-metadata-field-keywords = Keywords
pdf-metadata-close = Close
pdf-save = Save
pdf-save-confirm = The original file will be overwritten (a .bak backup is created automatically). Continue?
pdf-save-success = Saved to the original file (backup: { $backup })
pdf-export = Export as New...
