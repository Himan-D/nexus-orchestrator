"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { useParams } from "next/navigation";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  api,
  formatDuration,
  RUN_STATUS_CLASSES,
  TASK_STATUS_CLASSES,
  type RunDetail,
  type RunStatus,
  type TaskStatus,
} from "@/lib/api";

function fmtTime(t: string | null): string {
  return t ? new Date(t).toLocaleTimeString() : "—";
}

export default function RunDetailPage() {
  const params = useParams<{ id: string }>();
  const [run, setRun] = useState<RunDetail | null>(null);
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await api<RunDetail>(`/runs/${params.id}`);
      setRun(data);
      return data;
    } catch {
      return null;
    }
  }, [params.id]);

  useEffect(() => {
    let alive = true;
    let interval: ReturnType<typeof setInterval> | undefined;

    const tick = async () => {
      const data = await load();
      if (alive && data && (data.status === "queued" || data.status === "running")) {
        if (!interval) interval = setInterval(tick, 2000);
      } else if (interval) {
        clearInterval(interval);
      }
    };
    tick();
    return () => {
      alive = false;
      if (interval) clearInterval(interval);
    };
  }, [load]);

  const cancel = async () => {
    try {
      await api(`/runs/${params.id}/cancel`, { method: "POST" });
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to cancel");
    }
  };

  if (!run) {
    return (
      <div className="mx-auto max-w-7xl p-8">
        <p className="text-muted-foreground">{error ?? "Loading…"}</p>
      </div>
    );
  }

  const active = run.status === "queued" || run.status === "running";

  return (
    <div className="mx-auto max-w-7xl p-8 flex flex-col gap-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-bold tracking-tight">
              Run {run.id.substring(0, 8)}
            </h1>
            <Badge variant="outline" className={RUN_STATUS_CLASSES[run.status as RunStatus]}>
              {run.status}
            </Badge>
          </div>
          <p className="text-muted-foreground text-sm mt-1">
            Pipeline{" "}
            <Link href={`/jobs/${run.job_id}`} className="text-primary hover:underline">
              {run.job_name}
            </Link>
            {run.error && <span className="text-destructive"> — {run.error}</span>}
          </p>
        </div>
        {active && (
          <Button variant="destructive" onClick={cancel}>
            Cancel run
          </Button>
        )}
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Tasks</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {run.tasks.map((t) => (
            <div key={t.id} className="rounded-lg border p-3 space-y-2">
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2 min-w-0">
                  <Badge variant="outline" className={TASK_STATUS_CLASSES[t.status as TaskStatus]}>
                    {t.status}
                  </Badge>
                  <span className="font-medium text-sm truncate">{t.name}</span>
                  {t.attempts > 1 && (
                    <span className="text-xs text-amber-500">attempt {t.attempts}</span>
                  )}
                </div>
                <div className="flex items-center gap-3 text-xs text-muted-foreground shrink-0">
                  {t.materializes && (
                    <Link
                      href="/assets"
                      className="font-mono text-purple-400 hover:underline"
                    >
                      → {t.materializes}
                    </Link>
                  )}
                  <span>
                    {fmtTime(t.started_at)} → {fmtTime(t.finished_at)}
                  </span>
                  <span>{formatDuration(t.started_at, t.finished_at)}</span>
                  {t.logs && (
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => setExpanded((e) => ({ ...e, [t.id]: !e[t.id] }))}
                    >
                      {expanded[t.id] ? "Hide logs" : "Logs"}
                    </Button>
                  )}
                </div>
              </div>
              {t.error && <p className="text-xs text-destructive">{t.error}</p>}
              {expanded[t.id] && t.logs && (
                <pre className="text-xs bg-black/40 rounded p-3 overflow-x-auto max-h-72 whitespace-pre-wrap border">
                  {t.logs}
                </pre>
              )}
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
