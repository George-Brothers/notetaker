import { useEffect, useState } from "react";

export interface SearchBarProps {
  query: string;
  /**
   * Called on every keystroke; the caller (useLibrary) is responsible for
   * debouncing before it actually hits `api.search`, keeping this
   * component presentational.
   */
  onSearch: (query: string) => void;
}

export function SearchBar({ query, onSearch }: SearchBarProps) {
  const [value, setValue] = useState(query);

  useEffect(() => {
    setValue(query);
  }, [query]);

  return (
    <div className="search-bar" role="search">
      <label htmlFor="library-search" className="sr-only">
        Search transcripts and summaries
      </label>
      <input
        id="library-search"
        type="search"
        placeholder="Search transcripts and summaries…"
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
          onSearch(e.target.value);
        }}
      />
    </div>
  );
}
