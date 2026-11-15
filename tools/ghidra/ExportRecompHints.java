// Export recompiler hints as JSON: function entries/names and resolved jump tables.
// The recompiler decodes machine code itself; these hints only seed function
// boundaries and switch-table targets, and every hint is re-validated.
// Usage: analyzeHeadless <proj> <name> -process <prog> -noanalysis -readOnly \
//          -scriptPath tools/ghidra -postScript ExportRecompHints.java <out.json>
//@category Export
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class ExportRecompHints extends GhidraScript {
    @Override
    public void run() throws Exception {
        String out = getScriptArgs()[0];
        Listing listing = currentProgram.getListing();
        StringBuilder sb = new StringBuilder();
        sb.append("{\n  \"program_sha256\": \"").append(currentProgram.getExecutableSHA256()).append("\",\n");
        sb.append("  \"functions\": [\n");
        boolean first = true;
        for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
            if (f.isExternal()) continue;
            if (!first) sb.append(",\n");
            first = false;
            sb.append("    {\"entry\": ").append(f.getEntryPoint().getOffset())
              .append(", \"name\": \"").append(f.getName().replace("\"", "'")).append("\"")
              .append(", \"thunk\": ").append(f.isThunk()).append("}");
        }
        sb.append("\n  ],\n  \"jump_tables\": [\n");
        first = true;
        InstructionIterator it = listing.getInstructions(true);
        while (it.hasNext()) {
            Instruction ins = it.next();
            FlowType ft = ins.getFlowType();
            if (!(ft.isJump() && ft.isComputed())) continue;
            TreeSet<Long> targets = new TreeSet<>();
            for (Reference r : ins.getReferencesFrom()) {
                if (r.getReferenceType().isFlow()) targets.add(r.getToAddress().getOffset());
            }
            if (targets.isEmpty()) continue;
            if (!first) sb.append(",\n");
            first = false;
            sb.append("    {\"insn\": ").append(ins.getAddress().getOffset()).append(", \"targets\": [");
            StringJoiner j = new StringJoiner(", ");
            for (Long t : targets) j.add(Long.toString(t));
            sb.append(j).append("]}");
        }
        sb.append("\n  ]\n}\n");
        try (PrintWriter w = new PrintWriter(new FileWriter(out))) { w.print(sb); }
        println("hints written");
    }
}
