import type { CommandMap, ProjectListPage } from "./contracts.ts";

type Params = CommandMap["projects_list"]["params"];
type FetchPage = (params: Params, options: { signal: AbortSignal }) => Promise<ProjectListPage>;
export interface ProjectSearchState extends ProjectListPage {
  query: string;
  loading: boolean;
  error: unknown;
}

// One controller per consumer. Invalidate immediately on input (including IME
// composition), even when an earlier native request cannot actually be cancelled.
export class ProjectSearchController {
  private generation = 0;
  private pending?: AbortController;
  private disposed = false;
  private composing = false;
  private state: ProjectSearchState = { query: "", items: [], nextCursor: null, loading: false, error: null };

  private readonly fetchPage: FetchPage;
  private readonly publish: (state: ProjectSearchState) => void;
  constructor(fetchPage: FetchPage, publish: (state: ProjectSearchState) => void) {
    this.fetchPage = fetchPage;
    this.publish = publish;
  }

  async search(query: string, composing = false): Promise<void> {
    if (this.disposed) return;
    this.generation++;
    this.pending?.abort();
    this.composing = composing;
    this.state = { query, items: [], nextCursor: null, loading: false, error: null };
    this.publish(this.state);
    if (!composing) await this.request(false);
  }

  async loadMore(): Promise<void> {
    if (!this.disposed && !this.composing && !this.state.loading && this.state.nextCursor) await this.request(true);
  }

  dispose(): void {
    this.disposed = true;
    this.generation++;
    this.pending?.abort();
  }

  private async request(append: boolean): Promise<void> {
    const generation = this.generation;
    const previous = this.state;
    const pending = new AbortController();
    this.pending = pending;
    this.state = { ...previous, loading: true, error: null };
    this.publish(this.state);
    try {
      const page = await this.fetchPage({ query: previous.query, ...(append ? { cursor: previous.nextCursor! } : {}) }, { signal: pending.signal });
      if (this.disposed || generation !== this.generation) return;
      this.state = { query: previous.query, items: append ? [...previous.items, ...page.items] : page.items, nextCursor: page.nextCursor, loading: false, error: null };
    } catch (error) {
      if (this.disposed || generation !== this.generation) return;
      this.state = { ...previous, loading: false, error };
    }
    this.publish(this.state);
  }
}
