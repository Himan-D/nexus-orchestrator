// Thin typed client for the Nexus core API. All requests go through the
// Next.js rewrite (`/api/*` → Rust core), so cookies stay same-origin.

export type RunStatus = "queued" | "running" | "success" | "failed" | "cancelled";
export type TaskStatus = "pending" | "running" | "success" | "failed" | "skipped" | "cancelled";

export type Task = {
  id: string;
  name: string;
  command: string;
  depends_on: string[];
  asset_inputs: string[];
  materializes?: string | null;
  timeout_secs: number;
  max_retries: number;
  retry_delay_secs: number;
};

export type Job = {
  id: string;
  name: string;
  description: string;
  tasks: Task[];
  enabled: boolean;
  schedule: string | null;
  created_at: string;
  updated_at: string;
};

export type Run = {
  id: string;
  job_id: string;
  job_name: string;
  status: RunStatus;
  error: string | null;
  created_at: string;
  started_at: string | null;
  finished_at: string | null;
};

export type TaskRun = {
  id: string;
  task_id: string;
  name: string;
  status: TaskStatus;
  attempts: number;
  materializes: string | null;
  logs: string | null;
  error: string | null;
  started_at: string | null;
  finished_at: string | null;
};

export type Asset = {
  key: string;
  name: string;
  description: string;
  last_run_id: string | null;
  last_materialized_at: string | null;
  stale: boolean;
};

export type LineageNode = {
  key: string;
  layer: number;
  last_materialized_at: string | null;
  stale: boolean;
};

export type LineageEdge = { from: string; to: string };
export type LineageGraph = { nodes: LineageNode[]; edges: LineageEdge[] };

export type User = { id: string; email: string };
export type ListResponse<T> = { items: T[]; total: number | null };
export type Stats = {
  jobs: number;
  assets: number;
  runs_by_status: Record<RunStatus, number>;
};
export type RunDetail = Run & { tasks: TaskRun[] };

export class ApiError extends Error {
  code: string;
  status: number;
  constructor(message: string, code: string, status: number) {
    super(message);
    this.code = code;
    this.status = status;
  }
}

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`/api/v1${path}`, {
    credentials: "include",
    ...init,
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
  });

  if (res.status === 401 && !path.startsWith("/auth/")) {
    if (typeof window !== "undefined") window.location.href = "/login";
    throw new ApiError("Session expired", "unauthorized", 401);
  }

  const body = res.status === 204 ? null : await res.json().catch(() => null);
  if (!res.ok) {
    const err = body?.error;
    throw new ApiError(err?.message ?? res.statusText, err?.code ?? "error", res.status);
  }
  return body as T;
}

export const RUN_STATUS_CLASSES: Record<RunStatus, string> = {
  queued: "border-border bg-muted text-muted-foreground",
  running: "border-sky-500/30 bg-sky-500/10 text-sky-500",
  success: "border-emerald-500/30 bg-emerald-500/10 text-emerald-500",
  failed: "border-red-500/30 bg-red-500/10 text-red-500",
  cancelled: "border-amber-500/30 bg-amber-500/10 text-amber-500",
};

export const TASK_STATUS_CLASSES: Record<TaskStatus, string> = {
  pending: "border-border bg-muted text-muted-foreground",
  running: "border-sky-500/30 bg-sky-500/10 text-sky-500",
  success: "border-emerald-500/30 bg-emerald-500/10 text-emerald-500",
  failed: "border-red-500/30 bg-red-500/10 text-red-500",
  skipped: "border-zinc-500/30 bg-zinc-500/10 text-zinc-400",
  cancelled: "border-amber-500/30 bg-amber-500/10 text-amber-500",
};

export function formatDuration(startedAt: string | null, finishedAt: string | null): string {
  if (!startedAt || !finishedAt) return "—";
  const ms = new Date(finishedAt).getTime() - new Date(startedAt).getTime();
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const m = Math.floor(ms / 60_000);
  return `${m}m ${Math.round((ms % 60_000) / 1000)}s`;
}
