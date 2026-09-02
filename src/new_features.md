# New Features — Konfi v5.1.0+

## Parent Folder Inheritance for Marks

When you bookmark or tag an item (file/folder), all parent directories up the tree are **automatically marked with bookmarks**. This allows for quick navigation and filtering of related content.

### How It Works

1. **Bookmark an item**: Press `b` to toggle bookmark
   - The selected item gets bookmarked
   - All parent folders get bookmarked automatically
   - Status shows: "Added (parents marked)."

2. **Tag an item**: Press `h` to add a host tag
   - Enter the tag name when prompted
   - The selected item gets tagged
   - All parent folders get bookmarked (to show they contain tagged content)
   - Status shows: "Tag added (parents marked)."

3. **Add notes**: Press `n` to add/edit notes
   - Enter or edit the note text
   - Press Enter to save
   - Parent folders are NOT automatically marked for notes (only for bookmarks/tags)

### Benefits

- **Quick navigation**: Filter to bookmarks (`Alt+B`) to see all marked content and its parent structure
- **Better organization**: Parent folders automatically reflect that they contain important items
- **Visual hierarchy**: When browsing, parent folders show they contain bookmarked/tagged items

### Implementation Details

- Uses `ops::propagate_bookmarks_to_parents()` helper
- Walks up directory tree from the selected item
- Marks each parent with a bookmark automatically
- Works recursively to root (or until parent path is empty)
- Consolidation: Uses unified helper function to avoid code duplication
