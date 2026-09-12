"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Activity, CheckCircle2, Database, ListTree, XCircle, Clock } from "lucide-react";
import {
  api,
  formatDuration,
  RUN_STATUS_CLASSES,
  type Run,
  type RunStatus,
  type Stats,
} from "@/lib/api";

export default function Dashboard() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [runs, setRuns] = useState<Run[]>([]);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const [s, r] = await Promise.all([
          api<Stats>("/stats"),
          api<{ items: Run[] }>("/runs?limit=10"),
        ]);
        if (alive) {
          setStats(s);
          setRuns(r.items);
        }
      } catch {
        /* transient poll errors are non-fatal */
      }
    };
    tick();
    const interval = setInterval(tick, 3000);
    return () => {
      alive = false;
      clearInterval(interval);
    };
  }, []);

  const cards = [
    { label: "Queued", value: stats?.runs_by_status.queued ?? 0, icon: Clock, cls: "text-muted-foreground" },
    { label: "Running", value: stats?.runs_by_status.running ?? 0, icon: Activity, cls: "text-sky-500" },
    { label: "Succeeded", value: stats?.runs_by_status.success ?? 0, icon: CheckCircle2, cls: "text-emerald-500" },
    { label: "Failed", value: stats?.runs_by_status.failed ?? 0, icon: XCircle, cls: "text-destructive" },
    { label: "Pipelines", value: stats?.jobs ?? 0, icon: ListTree, cls: "text-primary" },
    { label: "Assets", value: stats?.assets ?? 0, icon: Database, cls: "text-purple-500" },
  ];

  return (
    <div className="mx-auto max-w-7xl p-8 flex flex-col gap-8">
      <div>
        <h1 className="text-2xl font-bold tracking-tight">Overview</h1>
        <p className="text-muted-foreground text-sm">Live view of the orchestration engine.</p>
      </div>

      <div className="grid grid-cols-2 md:grid-cols-6 gap-4">
        {cards.map((c) => (
          <Card key={c.label}>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-xs font-medium text-muted-foreground">{c.label}</CardTitle>
              <c.icon className={`h-4 w-4 ${c.cls}`} />
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold">{c.value}</div>
            </CardContent>
          </Card>
        ))}
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Recent runs</CardTitle>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Run</TableHead>
                <TableHead>Pipeline</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Duration</TableHead>
                <TableHead className="text-right">Created</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {runs.length === 0 && (
                <TableRow>
                  <TableCell colSpan={5} className="text-center h-24 text-muted-foreground">
                    No runs yet.{" "}
                    <Link href="/jobs" className="text-primary hover:underline">
                      Create a pipeline
                    </Link>{" "}
                    to get started.
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
                  <TableCell className="font-medium">{r.job_name}</TableCell>
                  <TableCell>
                    <Badge variant="outline" className={RUN_STATUS_CLASSES[r.status as RunStatus]}>
                      {r.status}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-muted-foreground text-xs">
                    {formatDuration(r.started_at, r.finished_at)}
                  </TableCell>
                  <TableCell className="text-right text-muted-foreground text-xs">
                    {new Date(r.created_at).toLocaleString()}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
