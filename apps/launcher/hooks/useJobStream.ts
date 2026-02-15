'use client';

import { useEffect, useMemo, useState } from 'react';
import { JobSummary } from '@/types/api';
import { createJobSocket, JobSocketStatus, JobUpdateMessage } from '@/lib/wsClient';

function mergeJobMessage(target: JobSummary | undefined, payload: JobUpdateMessage): JobSummary {
  const jobId = payload.jobId ?? payload.channel.replace(/^job:/, '');
  return {
    jobId,
    title: payload.title ?? target?.title ?? jobId,
    status: payload.status ?? target?.status ?? 'queued',
    message: payload.message ?? target?.message,
    updatedAt: payload.updatedAt ?? target?.updatedAt ?? new Date().toISOString(),
    progress: payload.progress ?? target?.progress,
  };
}

export function useJobStream(jobIds: string[] = []) {
  const [status, setStatus] = useState<JobSocketStatus>('idle');
  const [jobs, setJobs] = useState<Record<string, JobSummary>>({});
  const { jobKey, memoizedChannels } = useMemo(() => {
    const uniqueJobIds = Array.from(new Set(jobIds.filter(Boolean)));
    return {
      jobKey: uniqueJobIds.join(','),
      memoizedChannels: uniqueJobIds,
    };
  }, [jobIds]);

  useEffect(() => {

    if (!jobKey) {
      return undefined;
    }

    const controller = createJobSocket({
      onMessage(message) {
        if (!message.channel?.startsWith('job:')) return;
        setJobs((prev) => {
          const jobId = message.jobId ?? message.channel.replace(/^job:/, '');
          return {
            ...prev,
            [jobId]: mergeJobMessage(prev[jobId], message),
          };
        });
      },
      onStatus(next) {
        setStatus(next);
      },
    });

    if (!controller) {
      const scheduleError = () => setStatus('error');
      if (typeof queueMicrotask === 'function') {
        queueMicrotask(scheduleError);
      } else {
        setTimeout(scheduleError, 0);
      }
      return undefined;
    }

    memoizedChannels.forEach((id) => controller.subscribe(`job:${id}`));

    return () => {
      controller.close();
      setJobs({});
    };
  }, [jobKey, memoizedChannels]);

  const jobList = useMemo(() => Object.values(jobs), [jobs]);

  return { jobs: jobList, status };
}
