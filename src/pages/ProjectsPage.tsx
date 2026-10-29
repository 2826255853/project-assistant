import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import { call, createProjectSearch } from "../ipc/client";
import type { GitSummary, ProjectView, SettingsView } from "../ipc/contracts";
import type { ProjectSearchState } from "../ipc/projectSearch";
import { ErrorNotice } from "../components/ErrorNotice";
import { StatePanel } from "../components/StatePanel";
import { toPublicError } from "../ipc/errors";
import type { PublicError } from "../ipc/errors";
import "./ProjectsPage.css";

type ProjectAction = "directory" | "pick" | "file" | "remove" | null;

const initialSearch: ProjectSearchState = {
  query: "",
  items: [],
  nextCursor: null,
  loading: true,
  error: null,
};

function projectLabel(project: ProjectView) {
  return project.displayName || project.name;
}

function availabilityLabel(project: ProjectView) {
  if (project.authorization !== "granted") return "需要重新授权";
  if (project.availability === "available") return "可用";
  if (project.availability === "missing") return "路径失效";
  if (project.availability === "inaccessible") return "无权访问";
  return "身份已变化";
}

function gitLabel(summary: GitSummary | null) {
  if (!summary) return "正在读取 Git 状态";
  if (summary.state === "available") {
    const branch = summary.detached ? "分离 HEAD" : summary.branch || "未命名分支";
    return `${branch} · ${summary.workingTreeModified ? "工作区有修改" : "工作区干净"}`;
  }
  if (summary.state === "not_repository") return "不是 Git 仓库";
  if (summary.state === "git_unavailable") return "Git 不可用";
  if (summary.state === "timed_out") return "Git 查询超时";
  return "Git 摘要暂不可用";
}

function formatUsedAt(value: number | null) {
  if (value === null) return "尚未打开";
  return new Date(value).toLocaleString();
}

function RecentProjects({ onOpen, onManage, refreshKey }: { onOpen: (id: string) => void; onManage: () => void; refreshKey: string }) {
  const [items, setItems] = useState<ProjectView[]>([]);
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [error, setError] = useState<PublicError | null>(null);
  const request = useRef(0);
  const lastRefresh = useRef(refreshKey);

  useEffect(() => {
    const id = ++request.current;
    void call("projects_list", { limit: 5, sort: "recent" }).then(page => {
      if (id !== request.current) return;
      setItems(page.items.filter(project => project.lastUsedAt !== null));
      setPhase("ready");
      setError(null);
    }).catch(cause => {
      if (id !== request.current) return;
      setPhase("error");
      setError(toPublicError(cause));
    });
    return () => { ++request.current; };
  }, []);

  useEffect(() => {
    if (lastRefresh.current === refreshKey) return;
    lastRefresh.current = refreshKey;
    const id = ++request.current;
    void call("projects_list", { limit: 5, sort: "recent" }).then(page => {
      if (id !== request.current) return;
      setItems(page.items.filter(project => project.lastUsedAt !== null));
      setPhase("ready");
      setError(null);
    }).catch(cause => {
      if (id !== request.current) return;
      setPhase("error");
      setError(toPublicError(cause));
    });
  }, [refreshKey]);

  return <section className="recent-projects" data-testid="recent-projects">
    <div className="section-heading">
      <div><p className="eyebrow">近期入口</p><h3>最近项目</h3></div>
      <button type="button" className="quiet-button" onClick={onManage}>管理项目</button>
    </div>
    {phase === "loading" && <p className="inline-status" role="status" aria-busy="true">正在读取近期项目…</p>}
    {phase === "error" && error && <ErrorNotice error={error} testId="recent-projects-error" actions={<button type="button" onClick={onManage}>打开项目页重试</button>} />}
    {phase === "ready" && !items.length && <div className="empty-inline">还没有登记项目。打开项目页，从系统目录选择器添加第一个项目。</div>}
    {phase === "ready" && items.length > 0 && <div className="recent-project-list" role="list">
      {items.map(project => <button type="button" role="listitem" className="recent-project-row" key={project.id} onClick={() => onOpen(project.id)} data-testid="recent-project-row">
        <span className="project-row-mark" aria-hidden="true" />
        <span className="project-row-copy"><strong>{projectLabel(project)}</strong><small>{project.absolutePath}</small></span>
        <span className={`project-badge project-${project.availability}`}>{availabilityLabel(project)}</span>
      </button>)}
    </div>}
  </section>;
}

export function HomeProjects({ onOpen, onManage, refreshKey }: { onOpen: (id: string) => void; onManage: () => void; refreshKey: string }) {
  return <RecentProjects onOpen={onOpen} onManage={onManage} refreshKey={refreshKey} />;
}

export function ProjectsPage({ initialProjectId, onBackHome, refreshKey }: { initialProjectId: string | null; onBackHome: () => void; refreshKey: string }) {
  const [search, setSearch] = useState<ProjectSearchState>(initialSearch);
  const [query, setQuery] = useState("");
  const queryRef = useRef("");
  const composing = useRef(false);
  const controllerRef = useRef<ReturnType<typeof createProjectSearch> | null>(null);
  const rowRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const lastRefresh = useRef(refreshKey);
  const [selectedId, setSelectedId] = useState<string | null>(initialProjectId);
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const [settingsError, setSettingsError] = useState<PublicError | null>(null);
  const [sortBusy, setSortBusy] = useState(false);
  const [addBusy, setAddBusy] = useState(false);
  const [action, setAction] = useState<ProjectAction>(null);
  const [actionError, setActionError] = useState<PublicError | null>(null);
  const [actionMessage, setActionMessage] = useState("");
  const [git, setGit] = useState<GitSummary | null>(null);
  const [gitError, setGitError] = useState<PublicError | null>(null);
  const [gitBusy, setGitBusy] = useState(false);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [isComposing, setIsComposing] = useState(false);

  const selected = useMemo(() => search.items.find(project => project.id === selectedId) ?? null, [search.items, selectedId]);
  const sort = (settings?.values.find(change => change.key === "project_sort")?.value as "recent" | "name" | undefined) ?? "recent";
  const usable = selected?.availability === "available" && selected.authorization === "granted";
  const busy = Boolean(action) || addBusy || sortBusy;

  useEffect(() => {
    const controller = createProjectSearch(next => setSearch(next));
    controllerRef.current = controller;
    void controller.search("");
    return () => {
      controller.dispose();
      controllerRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (lastRefresh.current === refreshKey) return;
    lastRefresh.current = refreshKey;
    refreshList();
    if (settings) void call("settings_get", { scope: "global" }).then(setSettings).catch(cause => setSettingsError(toPublicError(cause)));
  }, [refreshKey]);

  useEffect(() => {
    let mounted = true;
    void call("settings_get", { scope: "global" }).then(view => {
      if (mounted) { setSettings(view); setSettingsError(null); }
    }).catch(cause => {
      if (mounted) setSettingsError(toPublicError(cause));
    });
    return () => { mounted = false; };
  }, []);

  useEffect(() => {
    if (!search.loading && !selectedId && initialProjectId && search.items.some(project => project.id === initialProjectId)) setSelectedId(initialProjectId);
    else if (!search.loading && !selectedId && !initialProjectId && search.items.length > 0) setSelectedId(search.items[0].id);
    if (!search.loading && selectedId && !search.items.some(project => project.id === selectedId)) {
      setSelectedId(null);
      setSelectedFile(null);
      setConfirmRemove(false);
    }
  }, [initialProjectId, search.items, selectedId]);

  useEffect(() => {
    if (!selected) {
      setGit(null);
      setGitError(null);
      return;
    }
    let mounted = true;
    setGit(null);
    setGitError(null);
    void call("project_git_summary", { projectId: selected.id }).then(summary => {
      if (mounted) setGit(summary);
    }).catch(cause => {
      if (mounted) setGitError(toPublicError(cause));
    });
    return () => { mounted = false; };
  }, [selected?.id, selected?.revision]);

  function refreshList() {
    void controllerRef.current?.search(queryRef.current);
  }

  function selectProject(id: string, focus = false) {
    setSelectedId(id);
    setSelectedFile(null);
    setConfirmRemove(false);
    if (focus) requestAnimationFrame(() => rowRefs.current[id]?.focus());
  }

  function handleListKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.target !== event.currentTarget) return;
    if (busy) return;
    if (!search.items.length || !["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const currentIndex = selectedId ? search.items.findIndex(project => project.id === selectedId) : -1;
    const next = event.key === "ArrowDown" ? currentIndex + 1 : event.key === "ArrowUp" ? (currentIndex < 0 ? 0 : currentIndex - 1) : event.key === "Home" ? 0 : search.items.length - 1;
    const bounded = Math.max(0, Math.min(search.items.length - 1, next));
    selectProject(search.items[bounded].id, true);
  }

  function handleSearch(value: string, isComposing = composing.current) {
    setQuery(value);
    setActionMessage("");
    queryRef.current = value;
    void controllerRef.current?.search(value, isComposing);
  }

  function handleRowKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const next = event.key === "ArrowDown" ? index + 1 : event.key === "ArrowUp" ? index - 1 : event.key === "Home" ? 0 : event.key === "End" ? search.items.length - 1 : -1;
    if (next < 0) return;
    event.preventDefault();
    const bounded = Math.max(0, Math.min(search.items.length - 1, next));
    selectProject(search.items[bounded].id, true);
  }

  async function addProject() {
    if (addBusy || action || sortBusy) return;
    setAddBusy(true);
    setActionError(null);
    try {
      const result = await call("projects_add_from_picker", {});
      if (result.outcome === "registered") {
        setSelectedId(result.project.id);
        setActionMessage("项目已添加到本机记录。");
        queryRef.current = "";
        setQuery("");
        await controllerRef.current?.search("");
      }
    } catch (cause) {
      setActionError(toPublicError(cause));
    } finally {
      setAddBusy(false);
    }
  }

  async function changeSort(value: "recent" | "name") {
    if (!settings || value === sort || sortBusy || action || addBusy) return;
    setSortBusy(true);
    setSettingsError(null);
    try {
      const next = await call("settings_update", {
        scope: "global",
        expectedRevision: settings.revision,
        changes: [{ key: "project_sort", value }],
      });
      setSettings(next);
      setActionMessage("项目排序已保存。");
      refreshList();
    } catch (cause) {
      const safe = toPublicError(cause);
      setSettingsError(safe);
      if (safe.code === "PA_CONFLICT_REVISION") {
        try { setSettings(await call("settings_get", { scope: "global" })); } catch { /* fixed conflict remains visible */ }
      }
    } finally {
      setSortBusy(false);
    }
  }

  async function refreshAfterAction(id: string) {
    try {
      await call("project_get", { projectId: id });
    } catch {
      // The list refresh below is authoritative and carries the fixed public error.
    }
    refreshList();
  }

  async function openDirectory() {
    if (!selected || !usable || action) return;
    setAction("directory");
    setActionError(null);
    try {
      await call("project_open_directory", { projectId: selected.id });
      setActionMessage("系统已接受目录打开请求。");
      await refreshAfterAction(selected.id);
    } catch (cause) {
      setActionError(toPublicError(cause));
      await refreshAfterAction(selected.id);
    } finally {
      setAction(null);
    }
  }

  async function pickFile() {
    if (!selected || !usable || action) return;
    setAction("pick");
    setActionError(null);
    try {
      const result = await call("project_pick_file", { projectId: selected.id });
      if ("outcome" in result) { setSelectedFile(null); setActionMessage("已取消文件选择。"); }
      else { setSelectedFile(result.relativePath); setActionMessage("文件已选中；打开操作会重新核验项目授权。"); }
    } catch (cause) {
      setActionError(toPublicError(cause));
      await refreshAfterAction(selected.id);
    } finally {
      setAction(null);
    }
  }

  async function openFile() {
    if (!selected || !usable || !selectedFile || action) return;
    setAction("file");
    setActionError(null);
    try {
      await call("project_open_file", { projectId: selected.id, relativePath: selectedFile });
      setActionMessage("系统已接受指定文件打开请求。");
      await refreshAfterAction(selected.id);
    } catch (cause) {
      setActionError(toPublicError(cause));
      await refreshAfterAction(selected.id);
    } finally {
      setAction(null);
    }
  }

  async function removeProject() {
    if (!selected || action) return;
    if (!confirmRemove) { setConfirmRemove(true); return; }
    setAction("remove");
    setActionError(null);
    try {
      await call("project_remove", { projectId: selected.id, expectedRevision: selected.revision });
      setActionMessage("项目授权记录已移除，项目文件保持不变。");
      setSelectedId(null);
      setSelectedFile(null);
      setConfirmRemove(false);
      refreshList();
    } catch (cause) {
      setActionError(toPublicError(cause));
      setConfirmRemove(false);
      await refreshAfterAction(selected.id);
    } finally {
      setAction(null);
    }
  }

  async function refreshGit() {
    if (!selected || gitBusy || action) return;
    setGitBusy(true);
    setGitError(null);
    try { setGit(await call("project_git_summary", { projectId: selected.id, refresh: true })); }
    catch (cause) { setGitError(toPublicError(cause)); }
    finally { setGitBusy(false); }
  }

  const showLoading = search.loading && search.items.length === 0;

  return <div className="page-content projects-page" data-testid="projects-page">
    <div className="projects-intro">
      <div><p className="eyebrow">项目工作区</p><h2>登记、搜索和打开本机项目</h2><p>项目授权只来自本机目录选择器。搜索只读取保存的项目元数据，Git 摘要失败不会阻塞项目操作。</p></div>
      <button type="button" className="primary-button" data-testid="project-add" disabled={busy} onClick={() => void addProject()}>{addBusy ? "正在选择…" : "添加项目"}</button>
    </div>
    <div className="projects-toolbar">
      <label className="project-search-field" htmlFor="project-search"><span>搜索项目</span><input id="project-search" data-testid="project-search" disabled={busy} value={query} placeholder="名称、显示名或路径" onChange={event => handleSearch(event.target.value)} onCompositionStart={event => { composing.current = true; setIsComposing(true); handleSearch(event.currentTarget.value, true); }} onCompositionEnd={event => { composing.current = false; setIsComposing(false); handleSearch(event.currentTarget.value, false); }} /></label>
      <label className="project-sort-field" htmlFor="project-sort-control"><span>排序</span><select id="project-sort-control" data-testid="project-sort" value={sort} disabled={!settings || busy} onChange={event => void changeSort(event.target.value as "recent" | "name")}><option value="recent">最近使用</option><option value="name">名称</option></select></label>
      <button type="button" className="quiet-button" data-testid="project-refresh" disabled={busy || search.loading} onClick={refreshList}>刷新</button>
    </div>
    {settingsError && <ErrorNotice error={settingsError} testId="project-settings-error" />}
    {actionError && <ErrorNotice error={actionError} testId="project-action-error" />}
    {actionMessage && <p className="project-action-message" role="status" data-testid="project-action-status">{actionMessage}</p>}
    {search.error !== null && <ErrorNotice error={search.error} testId="project-search-error" actions={<button type="button" onClick={refreshList}>重试搜索</button>} />}
    {isComposing && <p className="inline-status" role="status" data-testid="project-ime-status">正在输入法组合；提交文字后才会搜索。</p>}
    {showLoading && <StatePanel kind="loading" title="正在读取项目"><p>正在读取已登记的项目元数据…</p></StatePanel>}
    {!showLoading && !isComposing && !search.error && !search.items.length && <StatePanel kind="empty" title={query ? "没有匹配的项目" : "还没有登记项目"} actions={query ? <button type="button" className="quiet-button" onClick={() => handleSearch("")}>清除搜索</button> : <button type="button" className="primary-button" disabled={busy} onClick={() => void addProject()}>添加第一个项目</button>}><p>{query ? "请尝试名称、显示名或路径中的其他字符。" : "从系统目录选择器添加项目后，它会出现在这里。"}</p></StatePanel>}
    {search.items.length > 0 && <div className="projects-layout">
      <section className="project-list-panel" aria-label="项目列表">
        <div className="project-list-heading"><strong>{query ? `搜索结果（${search.items.length}${search.nextCursor ? "+" : ""}）` : "已登记项目"}</strong>{search.loading && <span role="status">正在更新…</span>}</div>
        <div className="project-list" role="listbox" tabIndex={0} aria-label="项目" aria-activedescendant={selectedId ? `project-row-${selectedId}` : undefined} onKeyDown={handleListKeyDown}>
          {search.items.map((project, index) => <button type="button" role="option" disabled={busy} aria-selected={project.id === selectedId} id={`project-row-${project.id}`} data-testid="project-row" className={`project-row${project.id === selectedId ? " selected" : ""}`} key={project.id} ref={element => { rowRefs.current[project.id] = element; }} onClick={() => selectProject(project.id)} onKeyDown={event => handleRowKeyDown(event, index)}>
            <span className="project-row-mark" aria-hidden="true" />
            <span className="project-row-copy"><strong>{projectLabel(project)}</strong><small>{project.absolutePath}</small><small>最近使用：{formatUsedAt(project.lastUsedAt)}</small></span>
            <span className={`project-badge project-${project.availability}`}>{availabilityLabel(project)}</span>
          </button>)}
        </div>
        {search.nextCursor && <button type="button" className="quiet-button project-load-more" disabled={busy || search.loading} onClick={() => void controllerRef.current?.loadMore()}>加载更多</button>}
      </section>
      {selected && <section className="project-detail-panel" aria-label="项目详情" data-testid="project-detail">
        <div className="project-detail-heading"><div><p className="eyebrow">已选择项目</p><h3>{projectLabel(selected)}</h3></div><span className={`project-badge project-${selected.availability}`}>{availabilityLabel(selected)}</span></div>
        <dl className="project-detail-list"><dt>目录</dt><dd>{selected.absolutePath}</dd><dt>授权</dt><dd>{selected.authorization === "granted" ? "已授权" : "需要重新授权"}</dd><dt>最近使用</dt><dd>{formatUsedAt(selected.lastUsedAt)}</dd><dt>Git</dt><dd>{gitLabel(git)} <button type="button" className="text-button" data-testid="project-git-refresh" disabled={gitBusy || busy} onClick={() => void refreshGit()}>{gitBusy ? "刷新中…" : "刷新"}</button></dd></dl>
        {gitError && <ErrorNotice error={gitError} testId="project-git-error" actions={<button type="button" onClick={() => void refreshGit()}>重试 Git 摘要</button>} />}
        {git && git.state !== "available" && <p className="project-degraded" data-testid="project-git-degraded">Git 摘要不可用不会阻塞目录和文件打开。</p>}
        {selected.availability !== "available" || selected.authorization !== "granted" ? <div className="project-unavailable" data-testid="project-path-unavailable">此项目路径当前不可用。请重新选择原目录并确认授权后恢复。<button type="button" className="quiet-button" data-testid="project-reauthorize" disabled={busy} onClick={() => void addProject()}>{addBusy ? "正在选择…" : "重新选择目录并授权"}</button></div> : <div className="project-actions">
          <button type="button" className="primary-button" data-testid="project-open-directory" disabled={busy} onClick={() => void openDirectory()}>{action === "directory" ? "正在打开…" : "打开目录"}</button>
          <button type="button" className="quiet-button" data-testid="project-pick-file" disabled={busy} onClick={() => void pickFile()}>{action === "pick" ? "正在选择…" : "选择文件"}</button>
          {selectedFile && <div className="selected-file" data-testid="project-selected-file"><span title={selectedFile}>{selectedFile}</span><button type="button" className="quiet-button" data-testid="project-open-file" disabled={busy} onClick={() => void openFile()}>{action === "file" ? "正在打开…" : "打开所选文件"}</button></div>}
        </div>}
        <div className="project-remove-area"><button type="button" className="quiet-button danger-button" data-testid="project-remove" disabled={busy} onClick={() => void removeProject()}>{action === "remove" ? "正在移除…" : confirmRemove ? "确认移除记录" : "移除项目记录"}</button>{confirmRemove && <button type="button" className="quiet-button" disabled={busy} onClick={() => setConfirmRemove(false)}>取消</button>}<small>只移除应用中的授权记录，不删除项目文件。</small></div>
      </section>}
    </div>}
    <div className="projects-footer"><button type="button" className="quiet-button" onClick={onBackHome}>返回概览</button><span>项目列表不会启动任务，也不会读取项目文件内容。</span></div>
  </div>;
}
