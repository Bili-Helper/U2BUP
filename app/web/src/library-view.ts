// A slow response from a previously selected library must never replace the current view.
export function createLibraryViewLoader<T>(
  fetchSnapshot: (libraryId: string | null) => Promise<T>,
  apply: (snapshot: T, libraryId: string | null) => void,
) {
  let revision = 0;
  return {
    invalidate() { revision++; },
    async load(libraryId: string | null) {
      const request = ++revision;
      const snapshot = await fetchSnapshot(libraryId);
      if (request !== revision) return false;
      apply(snapshot, libraryId);
      return true;
    },
  };
}
