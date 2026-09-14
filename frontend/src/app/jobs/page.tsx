"use client";

import { useCallback, useEffect, useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { api, ApiError, type Job } from "@/lib/api";

const SAMPLE_TASKS = `[
  {
    "id": "extract",
    "name": "Extract data",
    "command": "echo 'Fetching data...'; sleep 1; echo done",
    "depends_on": [],
    "asset_inputs": [],
    "materializes": "staging.raw_events",
    "timeout_secs": 300,
    "max_retries": 1,
    "retry_delay_secs": 5
  },
  {
    "id": "load",
    "name": "Load to warehouse",
    "command": "echo 'Loading...'; sleep 1; echo done",
    "depends_on": ["extract"],
    "asset_inputs": ["staging.raw_events"],
    "materializes": "warehouse.events"
  }
]`;

export default function JobsPage() {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [creating, setCreating] = useState(false);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [schedule, setSchedule] = useState("");
  const [tasksJson, setTasksJson] = useState(SAMPLE_TASKS);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const data = await api<{ items: Job[] }>("/jobs");
      setJobs(data.items);
    } catch {
      /* handled by auth redirect */
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const create = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    let tasks: unknown;
    try {
      tasks = JSON.parse(tasksJson);
    } catch {
      setError("Tasks must be valid JSON");
      return;
    }
    setCreating(true);
    try {
      await api("/jobs", {
        method: "POST",
        body: JSON.stringify({ name, description, schedule: schedule || null, tasks }),
      });
      setName("");
      setDescription("");
      setSchedule("");
      await load();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Failed to create pipeline");
    } finally {
      setCreating(false);
    }
  };

  return (
    <div className="mx-auto max-w-7xl p-8 flex flex-col gap-8">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">Pipelines</h1>
          <p className="text-muted-foreground text-sm">DAG definitions with tasks, dependencies, and assets.</p>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>New pipeline</CardTitle>
          <CardDescription>
            Tasks run in dependency order; independent tasks execute in parallel.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={create} className="space-y-4">
            <div className="grid gap-4 md:grid-cols-2">
              <div className="space-y-2">
                <label className="text-sm font-medium">Name</label>
                <Input
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="nightly_events_etl"
                  required
                />
              </div>
              <div className="space-y-2">
                <label className="text-sm font-medium">Description</label>
                <Input
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Ingest and warehouse events"
                />
              </div>
            </div>
            <div className="space-y-2 md:max-w-xs">
              <label className="text-sm font-medium">Schedule (cron, UTC — optional)</label>
              <Input
                value={schedule}
                onChange={(e) => setSchedule(e.target.value)}
                placeholder="*/30 * * * *"
              />
              <p className="text-xs text-muted-foreground">
                Standard 5-field cron. Empty = manual runs only.
              </p>
            </div>
            <div className="space-y-2">
              <label className="text-sm font-medium">Tasks (JSON)</label>
              <textarea
                value={tasksJson}
                onChange={(e) => setTasksJson(e.target.value)}
                spellCheck={false}
                className="w-full h-56 rounded-md border bg-muted/40 p-3 font-mono text-xs outline-none focus:ring-1 focus:ring-primary"
              />
            </div>
            {error && <p className="text-sm text-destructive">{error}</p>}
            <Button type="submit" disabled={creating}>
              {creating ? "Creating…" : "Create pipeline"}
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>All pipelines</CardTitle>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Tasks</TableHead>
                <TableHead>Assets produced</TableHead>
                <TableHead>Schedule</TableHead>
                <TableHead>State</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.length === 0 && (
                <TableRow>
                  <TableCell colSpan={5} className="text-center h-24 text-muted-foreground">
                    No pipelines yet.
                  </TableCell>
                </TableRow>
              )}
              {jobs.map((j) => (
                <TableRow key={j.id}>
                  <TableCell>
                    <Link href={`/jobs/${j.id}`} className="font-medium hover:text-primary hover:underline">
                      {j.name}
                    </Link>
                    {j.description && (
                      <p className="text-xs text-muted-foreground">{j.description}</p>
                    )}
                  </TableCell>
                  <TableCell>{j.tasks.length}</TableCell>
                  <TableCell className="font-mono text-xs text-muted-foreground">
                    {j.tasks.map((t) => t.materializes).filter(Boolean).join(", ") || "—"}
                  </TableCell>
                  <TableCell>
                    {j.schedule ? (
                      <span className="font-mono text-xs">{j.schedule}</span>
                    ) : (
                      <span className="text-muted-foreground text-xs">manual</span>
                    )}
                  </TableCell>
                  <TableCell>
                    <Badge variant={j.enabled ? "secondary" : "outline"}>
                      {j.enabled ? "active" : "paused"}
                    </Badge>
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
