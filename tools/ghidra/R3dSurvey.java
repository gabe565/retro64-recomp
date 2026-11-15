// Survey the boundary between game code and the r3d graphics wrapper.
// Writes <out_dir>/r3d_survey.json:
//   functions: every function in [r3d_lo, r3d_hi) with stack purge, ret immediates,
//              callers (split game / r3d / crt), callees, global reads/writes,
//              indirect calls, and r3d_* strings referenced
//   game_refs_to_r3d_globals: game-code instructions that read/write globals r3d writes
// Usage: analyzeHeadless <proj> <name> -process <prog> -noanalysis -readOnly \
//          -scriptPath tools/ghidra -postScript R3dSurvey.java <out_dir> <r3d_lo> <r3d_hi>
//@category Analysis
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import ghidra.program.model.data.*;
import java.io.*;
import java.util.*;

public class R3dSurvey extends GhidraScript {
    long lo, hi;
    static final long GAME_LO = 0x401000L, DATA_LO = 0x435000L, DATA_HI = 0xa0b000L;

    String region(long a) {
        if (a >= lo && a < hi) return "r3d";
        if (a >= GAME_LO && a < lo) return "game";
        return "crt";
    }

    static String esc(String s) { return s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n").replace("\r", "\\r").replace("\t", "\\t"); }

    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        String outDir = args[0];
        lo = Long.decode(args[1]);
        hi = Long.decode(args[2]);
        Listing listing = currentProgram.getListing();
        FunctionManager fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();

        Map<Long, Set<Long>> r3dWrites = new TreeMap<>();   // global -> writer functions
        StringBuilder fj = new StringBuilder();
        boolean first = true;
        for (Function f : fm.getFunctions(true)) {
            long e = f.getEntryPoint().getOffset();
            if (e < lo || e >= hi || f.isThunk()) continue;
            TreeSet<String> callersGame = new TreeSet<>(), callersR3d = new TreeSet<>(), callersCrt = new TreeSet<>();
            for (Function c : f.getCallingFunctions(monitor)) {
                long ca = c.getEntryPoint().getOffset();
                String r = region(ca);
                String key = String.format("0x%x", ca);
                if (r.equals("game")) callersGame.add(key); else if (r.equals("r3d")) callersR3d.add(key); else callersCrt.add(key);
            }
            TreeSet<String> callees = new TreeSet<>();
            for (Function c : f.getCalledFunctions(monitor)) callees.add(String.format("0x%x", c.getEntryPoint().getOffset()));
            TreeSet<String> reads = new TreeSet<>(), writes = new TreeSet<>(), strings = new TreeSet<>(), rets = new TreeSet<>();
            int indirect = 0;
            InstructionIterator it = listing.getInstructions(f.getBody(), true);
            while (it.hasNext()) {
                Instruction ins = it.next();
                FlowType ft = ins.getFlowType();
                if (ft.isCall() && ft.isComputed()) indirect++;
                if (ins.getMnemonicString().equalsIgnoreCase("RET")) {
                    rets.add(ins.getNumOperands() > 0 ? ins.getDefaultOperandRepresentation(0) : "0");
                }
                for (Reference r : ins.getReferencesFrom()) {
                    long t = r.getToAddress().getOffset();
                    if (!r.getToAddress().isMemoryAddress()) continue;
                    Data d = listing.getDataAt(r.getToAddress());
                    if (d != null && d.hasStringValue()) {
                        String v = String.valueOf(d.getValue());
                        if (v.startsWith("r3d_") || v.startsWith("_r3d_")) strings.add(v);
                    }
                    if (t >= 0x437000L && t < DATA_HI) {
                        String key = String.format("0x%x", t);
                        if (r.getReferenceType().isWrite()) {
                            writes.add(key);
                            r3dWrites.computeIfAbsent(t, k -> new TreeSet<>()).add(e);
                        } else if (r.getReferenceType().isRead() || r.getReferenceType().isData()) {
                            reads.add(key);
                        }
                    }
                }
            }
            if (!first) fj.append(",\n");
            first = false;
            fj.append(String.format("    {\"entry\": \"0x%x\", \"name\": \"%s\", \"size\": %d, \"stack_purge\": %d, \"cc\": \"%s\", \"params_guess\": %d, \"rets\": \"%s\", ",
                e, esc(f.getName()), f.getBody().getNumAddresses(), f.getStackPurgeSize(), esc(String.valueOf(f.getCallingConventionName())),
                f.getParameterCount(), String.join(",", rets)));
            fj.append(String.format("\"callers_game\": [%s], \"callers_r3d\": [%s], \"callers_crt\": [%s], \"callees\": [%s], \"indirect_calls\": %d, ",
                q(callersGame), q(callersR3d), q(callersCrt), q(callees), indirect));
            fj.append(String.format("\"global_reads\": [%s], \"global_writes\": [%s], \"r3d_strings\": [%s]}", q(reads), q(writes), q(strings)));
        }

        // Game code touching globals that r3d writes.
        StringBuilder gj = new StringBuilder();
        boolean gfirst = true;
        for (Map.Entry<Long, Set<Long>> en : r3dWrites.entrySet()) {
            Address ga = toAddr(en.getKey());
            TreeSet<String> gameReaders = new TreeSet<>(), gameWriters = new TreeSet<>();
            for (Reference r : rm.getReferencesTo(ga)) {
                long from = r.getFromAddress().getOffset();
                if (!region(from).equals("game")) continue;
                Function gf = fm.getFunctionContaining(r.getFromAddress());
                String key = String.format("0x%x@0x%x", from, gf == null ? 0 : gf.getEntryPoint().getOffset());
                if (r.getReferenceType().isWrite()) gameWriters.add(key); else gameReaders.add(key);
            }
            if (gameReaders.isEmpty() && gameWriters.isEmpty()) continue;
            if (!gfirst) gj.append(",\n");
            gfirst = false;
            TreeSet<String> w = new TreeSet<>();
            for (Long x : en.getValue()) w.add(String.format("0x%x", x));
            gj.append(String.format("    {\"global\": \"0x%x\", \"r3d_writers\": [%s], \"game_reads\": [%s], \"game_writes\": [%s]}",
                en.getKey(), q(w), q(gameReaders), q(gameWriters)));
        }

        try (PrintWriter pw = new PrintWriter(new FileWriter(outDir + "/r3d_survey.json"))) {
            pw.println("{\n  \"r3d_range\": [\"" + String.format("0x%x", lo) + "\", \"" + String.format("0x%x", hi) + "\"],");
            pw.println("  \"functions\": [\n" + fj + "\n  ],");
            pw.println("  \"game_refs_to_r3d_written_globals\": [\n" + gj + "\n  ]\n}");
        }
        println("r3d survey written");
    }

    static String q(Set<String> s) {
        StringJoiner j = new StringJoiner(", ");
        for (String x : s) j.add("\"" + esc(x) + "\"");
        return j.toString();
    }
}
