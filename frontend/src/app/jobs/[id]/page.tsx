"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  api,
  formatDuration,
  RUN_STATUS_CLASSES,
  type Job,
  type Run,
  type RunStatus,
} from "@/lib/api";

export default function JobDetailPage() {
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [job, setJob] = useState<Job | null>(null);
  const [runs, setRuns] = useState<Run[]>([]);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [j, r] = await Promise.all([
        api<Job>(`/jobs/${params.id}`),
        api<{ items: Run[] }>(`/jobs/${params.id}/runs?limit=20`),
      ]);
      setJob(j);
      setRuns(r.items);
    } catch {
      /* auth redirect handles 401 */
    }
  }, [params.id]);

  useEffect(() => {
    load();
    const interval = setInterval(load, 3000);
    return () => clearInterval(interval);
  }, [load]);

  const trigger = async () => {
    setError(null);
    try {
      const run = await api<Run>(`/jobs/${params.id}/runs`, { method: "POST" });
      router.push(`/runs/${run.id}`);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to trigger run");
    }
  };

  const toggleEnabled = async () => {
    if (!job) return;
    try {
      await api(`/jobs/${job.id}`, {
        method: "PATCH",
        body: JSON.stringify({ enabled: !job.enabled }),
      });
      await load();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to update pipeline");
    }
  };

  const remove = async () => {
    if (!job || !confirm(`Delete pipeline "${job.name}"? Run history is kept.`)) return;
    try {
      await api(`/jobs/${job.id}`, { method: "DELETE" });
      router.push("/jobs");
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to delete pipeline");
    }
  };

  if (!job) {
    return (
      <div className="mx-auto max-w-7xl p-8">
        <p className="text-muted-foreground">{error ?? "Loading…"}</p>
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-7xl p-8 flex flex-col gap-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-bold tracking-tight">{job.name}</h1>
            <Badge variant={job.enabled ? "secondary" : "outline"}>
              {job.enabled ? "active" : "paused"}
            </Badge>
          </div>
          {job.description && <p className="text-muted-foreground text-sm mt-1">{job.description}</p>}
        </div>
        <div className="flex gap-2">
          <Button onClick={trigger} disabled={!job.enabled}>
            Trigger run
          </Button>
          <Button variant="outline" onClick={toggleEnabled}>
            {job.enabled ? "Pause" : "Enable"}
          </Button>
          <Button variant="destructive" onClick={remove}>
            Delete
          </Button>
        </div>
      </div>
      {error && <p className="text-sm text-destructive">{error}</p>}

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Tasks ({job.tasks.length})</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {job.tasks.map((t) => (
              <div key={t.id} className="rounded-lg border p-3 space-y-1.5">
                <div className="flex items-center justify-between">
                  <span className="font-medium text-sm">{t.name}</span>
                  <span className="font-mono text-xs text-muted-foreground">{t.id}</span>
                </div>
                <pre className="text-xs bg-muted/50 rounded p-2 overflow-x-auto whitespace-pre-wrap">
                  {t.command}
                </pre>
                <div className="flex flex-wrap gap-2 text-xs text-muted-foreground">
                  {t.depends_on.length > 0 && <span>after: {t.depends_on.join(", ")}</span>}
                  {t.materializes && (
                    <span className="text-purple-400 font-mono">→ {t.materializes}</span>
                  )}
                  <span>timeout {t.timeout_secs}s</span>
                  {t.max_retries > 0 && <span>retries {t.max_retries}</span>}
                </div>
              </div>
            ))}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Runs</CardTitle>
          </CardHeader>
          <CardContent>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Run</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Duration</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {runs.length === 0 && (
                  <TableRow>
                    <TableCell colSpan={3} className="text-center h-20 text-muted-foreground">
                      Never run. Trigger one to see history here.
                    </TableCell>
                  </TableRow>
                )}
                {runs.map((r) => (
                  <TableRow key={r.id}>
                    <TableCell>
                      <Link href={`/runs/${r.id}`} className="font-mono text-xs text-primary hover:underline">
                        {r.id.substring(0, 8)}
                      </Link>
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline" className={RUN_STATUS_CLASSES[r.status as RunStatus]}>
                        {r.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="text-muted-foreground text-xs">
                      {formatDuration(r.started_at, r.finished_at)}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
