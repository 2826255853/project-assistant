import assert from "node:assert/strict";
import test from "node:test";
import { errorForCode } from "../../src/ipc/errors.ts";
import {
  isActiveProject, isGitSummary, isProjectFileParams, isProjectFilePickerResult, isProjectListPage,
  isProjectListParams, isProjectOpenAccepted, isProjectPickerResult, isProjectSettingsView,
  isProjectView, isRemovedProject, isSelectedProject,
  projectCommands, projectErrorCodes,
} from "../../src/ipc/projects.ts";

const projectId = "c14e7b09-b84c-44bc-9b57-a7182e70b056";
const project = {
  id: projectId, name: "项目 & tools", displayName: null, absolutePath: "C:\\work\\项目 & tools",
  availability: "available", authorization: "granted", revision: 1, createdAt: 10, lastUsedAt: null,
};

test("project views are bounded and reject paths as authorization data", () => {
  assert.equal(isProjectView(project), true);
  assert.equal(isProjectView({ ...project, pathIdentity: "synthetic-secret" }), false);
  assert.equal(isProjectView({ ...project, id: "not-a-uuid" }), false);
  assert.equal(isProjectView({ ...project, authorization: "granted", authorized: true }), false);
  assert.equal(isProjectListPage({ items: [project], nextCursor: null }), true);
  assert.equal(isProjectListPage({ items: Array.from({ length: 101 }, () => project), nextCursor: null }), false);
  assert.equal(isProjectListPage({ items: [project], nextCursor: "" }), false);
  assert.equal(isActiveProject({ projectId, name: project.name }), true);
});

test("project operation responses and inputs are strict and path-bounded", () => {
  assert.equal(isProjectPickerResult({ outcome: "cancelled" }), true);
  assert.equal(isProjectPickerResult({ outcome: "cancelled", project }), false);
  assert.equal(isProjectPickerResult({ outcome: "registered", project }), true);
  assert.equal(isSelectedProject({ activeProject: { projectId, name: "sample" } }), true);
  assert.equal(isRemovedProject({ removed: true, retainedForHistory: false }), true);
  assert.equal(isRemovedProject({ removed: false, retainedForHistory: false }), false);
  assert.equal(isProjectOpenAccepted({ systemAccepted: true }), true);
  assert.equal(isProjectOpenAccepted({ systemAccepted: false }), false);
  assert.equal(isProjectFilePickerResult({ outcome: "cancelled" }), true);
  assert.equal(isProjectFilePickerResult({ relativePath: "src/main.ts" }), true);
  assert.equal(isProjectFilePickerResult({ relativePath: "../outside.txt" }), false);
  assert.equal(isProjectFilePickerResult({ relativePath: "src/NUL.txt" }), false);
  assert.equal(isProjectFilePickerResult({ relativePath: "src/trailing. " }), false);
  assert.equal(isProjectListParams({}), true);
  assert.equal(isProjectListParams({ query: "中文", limit: 50 }), true);
  assert.equal(isProjectListParams({ limit: 0 }), false);
  assert.equal(isProjectListParams({ query: "bad\u0000input" }), false);
  assert.equal(isProjectFileParams({ projectId, relativePath: "src/main.ts" }), true);
  assert.equal(isProjectFileParams({ projectId, relativePath: "C:\\outside.txt" }), false);
});

test("Git degradation and project settings are typed data without raw output", () => {
  assert.equal(isGitSummary({ state: "available", isRepository: true, branch: "main", detached: false, workingTreeModified: true, refreshedAt: 100 }), true);
  assert.equal(isGitSummary({ state: "available", isRepository: true, branch: null, detached: true, workingTreeModified: false, refreshedAt: 100 }), true);
  assert.equal(isGitSummary({ state: "not_repository", isRepository: false, branch: null, detached: false, workingTreeModified: null, refreshedAt: 100 }), true);
  assert.equal(isGitSummary({ state: "git_unavailable", isRepository: null, branch: null, detached: false, workingTreeModified: null, refreshedAt: null }), true);
  assert.equal(isGitSummary({ state: "failed", isRepository: null, branch: null, detached: false, workingTreeModified: null, refreshedAt: null, stderr: "secret" }), false);
  assert.equal(isGitSummary({ state: "available", isRepository: true, branch: "x".repeat(513), detached: false, workingTreeModified: false, refreshedAt: 100 }), false);
  assert.equal(isGitSummary({ state: "timed_out", isRepository: null, branch: null, detached: false, workingTreeModified: null, refreshedAt: null }), true);
  assert.equal(isProjectSettingsView({ projectId, revision: 1, globalRevision: 2, values: [
    { key: "search_engine", value: "bing", source: "global" },
    { key: "ai_defaults", value: null, source: "project" },
  ] }), true);
  assert.equal(isProjectSettingsView({ projectId, revision: 1, globalRevision: 2, values: [
    { key: "approval_policy", value: "never", source: "project" },
  ] }), false);
  assert.equal(isProjectSettingsView({ projectId, revision: 1, globalRevision: 2, values: [
    { key: "search_engine", value: "bing", source: "global", path: "C:\\private" },
  ] }), false);
});

test("fixed project errors are local catalog entries", () => {
  assert.equal(projectErrorCodes.length, 11);
  for (const code of projectErrorCodes) assert.equal(errorForCode(code).code, code);
});

test("project DTO command catalog includes current and future commands", async () => {
  assert.deepEqual(projectCommands, [
    "projects_add_from_picker", "projects_list", "project_get", "project_select", "project_remove",
    "project_open_directory", "project_pick_file", "project_open_file", "project_git_summary",
  ]);
});
