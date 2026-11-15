// Export every non-thunk function's decompiled C plus a function index (TSV).
// Usage (headless): analyzeHeadless <proj_dir> <proj_name> -import <exe> \
//     -scriptPath tools/ghidra -postScript ExportDecomp.java <out_dir>
//@category Export
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import java.io.*;

public class ExportDecomp extends GhidraScript {
    @Override
    public void run() throws Exception {
        String outDir = getScriptArgs()[0];
        DecompInterface di = new DecompInterface();
        di.setOptions(new DecompileOptions());
        di.openProgram(currentProgram);
        try (PrintWriter c = new PrintWriter(new FileWriter(outDir + "/decomp.c"));
             PrintWriter idx = new PrintWriter(new FileWriter(outDir + "/functions.tsv"))) {
            idx.println("address\tname\tsize\tcallers\tcallees");
            int n = 0;
            FunctionIterator it = currentProgram.getFunctionManager().getFunctions(true);
            while (it.hasNext() && !monitor.isCancelled()) {
                Function f = it.next();
                idx.println(f.getEntryPoint() + "\t" + f.getName() + "\t" + f.getBody().getNumAddresses()
                        + "\t" + f.getCallingFunctions(monitor).size() + "\t" + f.getCalledFunctions(monitor).size());
                if (f.isExternal() || f.isThunk()) continue;
                DecompileResults r = di.decompileFunction(f, 120, monitor);
                c.println("// ===== " + f.getEntryPoint() + " " + f.getName() + " =====");
                if (r != null && r.decompileCompleted() && r.getDecompiledFunction() != null)
                    c.println(r.getDecompiledFunction().getC());
                else
                    c.println("// DECOMPILE FAILED: " + (r == null ? "null" : r.getErrorMessage()));
                n++;
            }
            println("Exported " + n + " functions");
        } finally {
            di.dispose();
        }
    }
}
