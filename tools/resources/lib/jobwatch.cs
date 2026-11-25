// Runs one command in a Windows Job Object and writes what its whole process tree used, as JSON.
//
//   jobwatch.exe --out <stats.json> [--cwd <dir>] [--stdout <file>] [--stderr <file>] [--interval-ms 100]
//                [--linger-ms 5000] [--affinity <cpu mask>] -- <program> [args...]
//
// tools/resources/lib/tree.mjs builds it on first use with the C# compiler that ships with Windows (.NET Framework 4,
// so C# 5: no string interpolation or newer syntax), into a tools folder outside the measured run.
//
// The command starts suspended, joins the job, then runs; every process it starts joins the job too (the job allows no
// breakaway, so a daemon that asks to leave it starts inside it). The job's completion port names each process as it
// starts and ends, so each one is opened at once and read again after it exits: its peak working set, peak private
// bytes, page faults, I/O, CPU time and handle count. Every --interval-ms the live processes are sampled for the summed
// working set and private bytes and their handle counts. At the end the job's own accounting gives the exact totals
// (CPU, page faults, I/O, processes) and its peak commit. Processes still running --linger-ms after the command
// exited are named in "leftRunning" and then ended with the job. --affinity holds every process of the tree to those
// CPUs (a decimal mask), so a run on a large machine uses as many as a CI runner has.
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

static class JobWatch
{
	const int JobObjectBasicAccountingInformation = 1;
	const int JobObjectBasicProcessIdList = 3;
	const int JobObjectAssociateCompletionPortInformation = 7;
	const int JobObjectBasicAndIoAccountingInformation = 8;
	const int JobObjectExtendedLimitInformation = 9;
	const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000;
	const uint JOB_OBJECT_LIMIT_AFFINITY = 0x10;
	const uint JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO = 4;
	const uint JOB_OBJECT_MSG_NEW_PROCESS = 6;
	const uint JOB_OBJECT_MSG_EXIT_PROCESS = 7;
	const uint JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS = 8;
	const uint CREATE_SUSPENDED = 0x4;
	const uint CREATE_UNICODE_ENVIRONMENT = 0x400;
	const uint PROCESS_QUERY_INFORMATION = 0x400;
	const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000;
	const uint PROCESS_VM_READ = 0x10;
	const uint SYNCHRONIZE = 0x100000;
	const uint GENERIC_READ = 0x80000000;
	const uint GENERIC_WRITE = 0x40000000;
	const uint FILE_SHARE_READ = 1, FILE_SHARE_WRITE = 2;
	const uint CREATE_ALWAYS = 2, OPEN_EXISTING = 3;
	const int STARTF_USESTDHANDLES = 0x100;
	static readonly IntPtr INVALID_HANDLE_VALUE = new IntPtr(-1);

	[StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
	struct STARTUPINFO
	{
		public int cb;
		public string lpReserved, lpDesktop, lpTitle;
		public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
		public short wShowWindow, cbReserved2;
		public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
	}

	[StructLayout(LayoutKind.Sequential)]
	struct PROCESS_INFORMATION
	{
		public IntPtr hProcess, hThread;
		public int dwProcessId, dwThreadId;
	}

	[StructLayout(LayoutKind.Sequential)]
	struct SECURITY_ATTRIBUTES
	{
		public int nLength;
		public IntPtr lpSecurityDescriptor;
		public int bInheritHandle;
	}

	[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateJobObjectW(IntPtr attributes, string name);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool SetInformationJobObject(IntPtr job, int infoClass, IntPtr info, uint length);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool QueryInformationJobObject(IntPtr job, int infoClass, IntPtr info, uint length, IntPtr returned);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateJobObject(IntPtr job, uint exitCode);
	[DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr CreateIoCompletionPort(IntPtr file, IntPtr existing, UIntPtr key, uint threads);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool GetQueuedCompletionStatus(IntPtr port, out uint bytes, out UIntPtr key, out IntPtr overlapped, uint milliseconds);
	[DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool CloseHandle(IntPtr handle);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool GetProcessTimes(IntPtr process, out long creation, out long exit, out long kernel, out long user);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool GetProcessIoCounters(IntPtr process, IntPtr counters);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool GetProcessHandleCount(IntPtr process, out uint count);
	[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool QueryFullProcessImageNameW(IntPtr process, uint flags, StringBuilder name, ref uint size);
	[DllImport("psapi.dll", SetLastError = true)] static extern bool GetProcessMemoryInfo(IntPtr process, IntPtr counters, uint size);
	[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessW(string application, StringBuilder commandLine, IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint flags, IntPtr environment, string directory, ref STARTUPINFO startup, out PROCESS_INFORMATION info);
	[DllImport("kernel32.dll", SetLastError = true)] static extern uint ResumeThread(IntPtr thread);
	[DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateFileW(string name, uint access, uint share, ref SECURITY_ATTRIBUTES attributes, uint disposition, uint flags, IntPtr template);
	[DllImport("kernel32.dll", SetLastError = true)] static extern bool TerminateProcess(IntPtr process, uint code);

	// What one process used, read while it runs and once more after it exits
	class Proc
	{
		public uint Pid;
		public IntPtr Handle;
		public string Image = "";
		public bool Exited;
		public long PeakWorkingSet, PeakPrivate, WorkingSet, Private, PageFaults;
		public long ReadBytes, WriteBytes, OtherBytes, ReadOps, WriteOps, OtherOps;
		public long UserTicks, KernelTicks;
		public long StartMs = -1, EndMs = -1;
		public uint Handles, PeakHandles;
	}

	static readonly Dictionary<uint, Proc> procs = new Dictionary<uint, Proc>();
	static readonly List<Proc> all = new List<Proc>();
	static long peakSumWorkingSet, peakSumPrivate, peakSumHandles;
	static int peakLive, unopened, samples;
	static IntPtr memInfo = Marshal.AllocHGlobal(80);
	static IntPtr ioInfo = Marshal.AllocHGlobal(48);
	static System.Diagnostics.Stopwatch clock = System.Diagnostics.Stopwatch.StartNew();

	static int Main(string[] argv)
	{
		string outPath = null, cwd = null, stdoutPath = null, stderrPath = null;
		uint interval = 100;
		long linger = 5000;
		long affinity = 0;
		int split = Array.IndexOf(argv, "--");
		if (split < 0 || split == argv.Length - 1) return Usage("missing -- <program> [args...]");
		for (int i = 0; i < split; i++)
		{
			string value = i + 1 < split ? argv[i + 1] : null;
			switch (argv[i])
			{
				case "--out": outPath = value; i++; break;
				case "--cwd": cwd = value; i++; break;
				case "--stdout": stdoutPath = value; i++; break;
				case "--stderr": stderrPath = value; i++; break;
				case "--interval-ms": interval = uint.Parse(value, CultureInfo.InvariantCulture); i++; break;
				case "--linger-ms": linger = long.Parse(value, CultureInfo.InvariantCulture); i++; break;
				case "--affinity": affinity = long.Parse(value, CultureInfo.InvariantCulture); i++; break;
				default: return Usage("unknown option " + argv[i]);
			}
		}
		if (outPath == null) return Usage("--out is required");
		var command = new string[argv.Length - split - 1];
		Array.Copy(argv, split + 1, command, 0, command.Length);

		IntPtr job = CreateJobObjectW(IntPtr.Zero, null);
		if (job == IntPtr.Zero) return Fail("CreateJobObject");
		// Kill on close: if this watcher dies, the measured tree dies with it instead of running on unwatched
		IntPtr limits = Marshal.AllocHGlobal(144);
		Zero(limits, 144);
		Marshal.WriteInt32(limits, 16, (int)(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | (affinity != 0 ? JOB_OBJECT_LIMIT_AFFINITY : 0)));
		Marshal.WriteInt64(limits, 48, affinity);
		if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, limits, 144)) return Fail("SetInformationJobObject(limits)");
		IntPtr port = CreateIoCompletionPort(INVALID_HANDLE_VALUE, IntPtr.Zero, UIntPtr.Zero, 1);
		if (port == IntPtr.Zero) return Fail("CreateIoCompletionPort");
		IntPtr association = Marshal.AllocHGlobal(16);
		Marshal.WriteIntPtr(association, 0, job);
		Marshal.WriteIntPtr(association, 8, port);
		if (!SetInformationJobObject(job, JobObjectAssociateCompletionPortInformation, association, 16)) return Fail("SetInformationJobObject(port)");

		var inherit = new SECURITY_ATTRIBUTES();
		inherit.nLength = Marshal.SizeOf(typeof(SECURITY_ATTRIBUTES));
		inherit.bInheritHandle = 1;
		var startup = new STARTUPINFO();
		startup.cb = Marshal.SizeOf(typeof(STARTUPINFO));
		startup.dwFlags = STARTF_USESTDHANDLES;
		startup.hStdInput = CreateFileW("NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, ref inherit, OPEN_EXISTING, 0, IntPtr.Zero);
		startup.hStdOutput = OpenOutput(stdoutPath, ref inherit);
		startup.hStdError = stderrPath != null && stderrPath == stdoutPath ? startup.hStdOutput : OpenOutput(stderrPath, ref inherit);
		PROCESS_INFORMATION info;
		var commandLine = new StringBuilder(CommandLine(command));
		long started = clock.ElapsedMilliseconds;
		if (!CreateProcessW(null, commandLine, IntPtr.Zero, IntPtr.Zero, true, CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT, IntPtr.Zero, cwd, ref startup, out info))
			return Fail("CreateProcess " + command[0]);
		if (!AssignProcessToJobObject(job, info.hProcess))
		{
			TerminateProcess(info.hProcess, 1);
			return Fail("AssignProcessToJobObject");
		}
		var root = Track((uint)info.dwProcessId, info.hProcess);
		ResumeThread(info.hThread);
		CloseHandle(info.hThread);

		bool zero = false, rootDone = false, terminated = false;
		long rootEnded = -1;
		var leftRunning = new List<Proc>();
		long nextSample = 0;
		while (!zero)
		{
			uint message;
			UIntPtr key;
			IntPtr overlapped;
			long now = clock.ElapsedMilliseconds;
			uint wait = now >= nextSample ? 0 : (uint)Math.Min(interval, nextSample - now);
			if (GetQueuedCompletionStatus(port, out message, out key, out overlapped, wait))
			{
				uint pid = (uint)overlapped.ToInt64();
				if (message == JOB_OBJECT_MSG_NEW_PROCESS)
				{
					if (!procs.ContainsKey(pid)) Track(pid, IntPtr.Zero);
				}
				else if (message == JOB_OBJECT_MSG_EXIT_PROCESS || message == JOB_OBJECT_MSG_ABNORMAL_EXIT_PROCESS)
				{
					Proc proc;
					if (procs.TryGetValue(pid, out proc)) Finish(proc);
				}
				else if (message == JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO) zero = true;
			}
			now = clock.ElapsedMilliseconds;
			if (now >= nextSample)
			{
				Sample();
				nextSample = now + interval;
			}
			if (!rootDone && root.Exited)
			{
				rootDone = true;
				rootEnded = now;
			}
			if (rootDone && !zero && !terminated && now - rootEnded >= linger)
			{
				foreach (var proc in all) if (!proc.Exited) leftRunning.Add(proc);
				TerminateJobObject(job, 1);
				terminated = true;
			}
		}
		long wall = (rootEnded >= 0 ? rootEnded : clock.ElapsedMilliseconds) - started;
		foreach (var proc in all) if (!proc.Exited) Finish(proc);
		uint exitCode;
		GetExitCodeProcess(root.Handle, out exitCode);

		IntPtr accounting = Marshal.AllocHGlobal(96);
		Zero(accounting, 96);
		QueryInformationJobObject(job, JobObjectBasicAndIoAccountingInformation, accounting, 96, IntPtr.Zero);
		Zero(limits, 144);
		QueryInformationJobObject(job, JobObjectExtendedLimitInformation, limits, 144, IntPtr.Zero);

		var json = new StringBuilder();
		json.Append("{\"tool\":\"jobwatch\",\"version\":1");
		Field(json, "exitCode", (long)exitCode);
		Field(json, "wallMs", wall);
		Field(json, "cpuUserMs", Marshal.ReadInt64(accounting, 0) / 10000);
		Field(json, "cpuKernelMs", Marshal.ReadInt64(accounting, 8) / 10000);
		Field(json, "pageFaults", (long)(uint)Marshal.ReadInt32(accounting, 32));
		Field(json, "processes", (long)(uint)Marshal.ReadInt32(accounting, 36));
		Field(json, "ioReadOps", Marshal.ReadInt64(accounting, 48));
		Field(json, "ioWriteOps", Marshal.ReadInt64(accounting, 56));
		Field(json, "ioOtherOps", Marshal.ReadInt64(accounting, 64));
		Field(json, "ioReadBytes", Marshal.ReadInt64(accounting, 72));
		Field(json, "ioWriteBytes", Marshal.ReadInt64(accounting, 80));
		Field(json, "ioOtherBytes", Marshal.ReadInt64(accounting, 88));
		Field(json, "peakPrivateMax", Marshal.ReadInt64(limits, 128));
		Field(json, "peakPrivateSum", Marshal.ReadInt64(limits, 136));
		long peakWs = 0;
		uint peakHandles = 0;
		foreach (var proc in all)
		{
			peakWs = Math.Max(peakWs, proc.PeakWorkingSet);
			peakHandles = Math.Max(peakHandles, proc.PeakHandles);
		}
		Field(json, "peakWorkingSetMax", peakWs);
		Field(json, "peakWorkingSetSum", peakSumWorkingSet);
		Field(json, "peakPrivateSumSampled", peakSumPrivate);
		Field(json, "peakHandlesMax", (long)peakHandles);
		Field(json, "peakHandlesSum", peakSumHandles);
		Field(json, "peakProcesses", (long)peakLive);
		Field(json, "unopenedProcesses", (long)unopened);
		Field(json, "samples", (long)samples);
		Field(json, "intervalMs", (long)interval);
		json.Append(",\"leftRunning\":[");
		for (int i = 0; i < leftRunning.Count; i++)
		{
			if (i > 0) json.Append(',');
			Text(json, Path.GetFileName(leftRunning[i].Image));
		}
		json.Append("],\"perProcess\":[");
		for (int i = 0; i < all.Count; i++)
		{
			var proc = all[i];
			if (i > 0) json.Append(',');
			json.Append("{\"pid\":").Append(proc.Pid).Append(",\"image\":");
			Text(json, Path.GetFileName(proc.Image));
			Field(json, "startMs", proc.StartMs - started);
			Field(json, "endMs", proc.EndMs < 0 ? -1 : proc.EndMs - started);
			Field(json, "peakWorkingSet", proc.PeakWorkingSet);
			Field(json, "peakPrivate", proc.PeakPrivate);
			Field(json, "pageFaults", proc.PageFaults);
			Field(json, "readBytes", proc.ReadBytes);
			Field(json, "writeBytes", proc.WriteBytes);
			Field(json, "otherBytes", proc.OtherBytes);
			Field(json, "cpuUserMs", proc.UserTicks / 10000);
			Field(json, "cpuKernelMs", proc.KernelTicks / 10000);
			Field(json, "peakHandles", (long)proc.PeakHandles);
			json.Append('}');
		}
		json.Append("]}");
		File.WriteAllText(outPath, json.ToString(), new UTF8Encoding(false));
		return (int)exitCode;
	}

	static Proc Track(uint pid, IntPtr handle)
	{
		var proc = new Proc();
		proc.Pid = pid;
		proc.StartMs = clock.ElapsedMilliseconds;
		if (handle == IntPtr.Zero) handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | SYNCHRONIZE, false, pid);
		if (handle == IntPtr.Zero) handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, false, pid);
		proc.Handle = handle;
		if (handle == IntPtr.Zero)
		{
			// gone before it could be opened: the job's totals still hold what it used
			unopened++;
			proc.Exited = true;
		}
		else
		{
			var name = new StringBuilder(1024);
			uint size = (uint)name.Capacity;
			if (QueryFullProcessImageNameW(handle, 0, name, ref size)) proc.Image = name.ToString();
			Read(proc);
		}
		procs[pid] = proc;
		all.Add(proc);
		return proc;
	}

	// Reads a process's counters: while it runs, and once after it exits (the handle keeps them)
	static void Read(Proc proc)
	{
		if (proc.Handle == IntPtr.Zero) return;
		Marshal.WriteInt32(memInfo, 0, 80);
		if (GetProcessMemoryInfo(proc.Handle, memInfo, 80))
		{
			proc.PageFaults = (uint)Marshal.ReadInt32(memInfo, 4);
			proc.PeakWorkingSet = Math.Max(proc.PeakWorkingSet, Marshal.ReadInt64(memInfo, 8));
			proc.WorkingSet = Marshal.ReadInt64(memInfo, 16);
			proc.PeakPrivate = Math.Max(proc.PeakPrivate, Marshal.ReadInt64(memInfo, 64));
			proc.Private = Marshal.ReadInt64(memInfo, 72);
		}
		if (GetProcessIoCounters(proc.Handle, ioInfo))
		{
			proc.ReadOps = Marshal.ReadInt64(ioInfo, 0);
			proc.WriteOps = Marshal.ReadInt64(ioInfo, 8);
			proc.OtherOps = Marshal.ReadInt64(ioInfo, 16);
			proc.ReadBytes = Marshal.ReadInt64(ioInfo, 24);
			proc.WriteBytes = Marshal.ReadInt64(ioInfo, 32);
			proc.OtherBytes = Marshal.ReadInt64(ioInfo, 40);
		}
		long creation, exit, kernel, user;
		if (GetProcessTimes(proc.Handle, out creation, out exit, out kernel, out user))
		{
			proc.KernelTicks = kernel;
			proc.UserTicks = user;
		}
		uint handles;
		if (!proc.Exited && GetProcessHandleCount(proc.Handle, out handles))
		{
			proc.Handles = handles;
			proc.PeakHandles = Math.Max(proc.PeakHandles, handles);
		}
	}

	static void Finish(Proc proc)
	{
		// a later process may get the same id
		Proc listed;
		if (procs.TryGetValue(proc.Pid, out listed) && listed == proc) procs.Remove(proc.Pid);
		if (proc.Handle == IntPtr.Zero) return;
		proc.Exited = true;
		proc.EndMs = clock.ElapsedMilliseconds;
		proc.WorkingSet = 0;
		proc.Private = 0;
		Read(proc);
		// the root's handle stays open for its exit code
		if (all.Count > 0 && all[0] != proc)
		{
			CloseHandle(proc.Handle);
			proc.Handle = IntPtr.Zero;
		}
	}

	static void Sample()
	{
		long sumWs = 0, sumPrivate = 0, sumHandles = 0;
		int live = 0;
		foreach (var proc in all)
		{
			if (proc.Exited || proc.Handle == IntPtr.Zero) continue;
			Read(proc);
			sumWs += proc.WorkingSet;
			sumPrivate += proc.Private;
			sumHandles += proc.Handles;
			live++;
		}
		samples++;
		peakSumWorkingSet = Math.Max(peakSumWorkingSet, sumWs);
		peakSumPrivate = Math.Max(peakSumPrivate, sumPrivate);
		peakSumHandles = Math.Max(peakSumHandles, sumHandles);
		peakLive = Math.Max(peakLive, live);
	}

	static IntPtr OpenOutput(string path, ref SECURITY_ATTRIBUTES inherit)
	{
		return CreateFileW(path ?? "NUL", GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, ref inherit, path == null ? OPEN_EXISTING : CREATE_ALWAYS, 0, IntPtr.Zero);
	}

	// The command line CommandLineToArgvW (and the C runtime) splits back into exactly these arguments
	static string CommandLine(string[] args)
	{
		var line = new StringBuilder();
		foreach (var arg in args)
		{
			if (line.Length > 0) line.Append(' ');
			if (arg.Length > 0 && arg.IndexOfAny(new[] { ' ', '\t', '\n', '\v', '"' }) < 0)
			{
				line.Append(arg);
				continue;
			}
			line.Append('"');
			int slashes = 0;
			foreach (char c in arg)
			{
				if (c == '\\') { slashes++; continue; }
				if (c == '"') line.Append('\\', slashes * 2 + 1);
				else line.Append('\\', slashes);
				slashes = 0;
				line.Append(c);
			}
			line.Append('\\', slashes * 2);
			line.Append('"');
		}
		return line.ToString();
	}

	static void Field(StringBuilder json, string name, long value)
	{
		json.Append(",\"").Append(name).Append("\":").Append(value.ToString(CultureInfo.InvariantCulture));
	}

	static void Text(StringBuilder json, string value)
	{
		json.Append('"');
		foreach (char c in value ?? "")
		{
			if (c == '"' || c == '\\') json.Append('\\').Append(c);
			else if (c < ' ') json.Append("\\u").Append(((int)c).ToString("x4"));
			else json.Append(c);
		}
		json.Append('"');
	}

	static void Zero(IntPtr memory, int length)
	{
		for (int i = 0; i < length; i++) Marshal.WriteByte(memory, i, 0);
	}

	static int Usage(string message)
	{
		Console.Error.WriteLine("jobwatch: " + message);
		Console.Error.WriteLine("usage: jobwatch --out <stats.json> [--cwd <dir>] [--stdout <file>] [--stderr <file>] [--interval-ms 100] [--linger-ms 5000] [--affinity <mask>] -- <program> [args...]");
		return 125;
	}

	static int Fail(string what)
	{
		Console.Error.WriteLine("jobwatch: " + what + " failed: Windows error " + Marshal.GetLastWin32Error());
		return 125;
	}
}
