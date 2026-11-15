// Survey a program for static-recompilation feasibility.
// Writes <out_dir>/recomp_survey.txt with: instruction-class counts, indirect
// branch inventory (resolved vs unresolved), x87/MMX/SSE usage, FPU control-word
// writes, fs: segment accesses (SEH), code bytes in .text outside any function,
// and per-function indirect-call sites.
// Usage: analyzeHeadless <proj> <name> -process <prog> -noanalysis \
//          -scriptPath tools/ghidra -postScript RecompSurvey.java <out_dir>
//@category Analysis
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.mem.*;
import ghidra.program.model.symbol.*;
import java.io.*;
import java.util.*;

public class RecompSurvey extends GhidraScript {
    @Override
    public void run() throws Exception {
        String outDir = getScriptArgs()[0];
        Listing listing = currentProgram.getListing();
        FunctionManager fm = currentProgram.getFunctionManager();
        ReferenceManager rm = currentProgram.getReferenceManager();
        MemoryBlock text = currentProgram.getMemory().getBlock(".text");

        Map<String, Integer> mnem = new TreeMap<>();
        int insns = 0, x87 = 0, mmx = 0, sse = 0, fldcw = 0, fsSeg = 0, rep = 0;
        int indJmp = 0, indJmpResolved = 0, indCall = 0, indCallViaImport = 0, indCallResolved = 0;
        List<String> unresolvedJmp = new ArrayList<>();
        Map<String, Integer> indCallsByFunc = new TreeMap<>();
        Set<String> x87Funcs = new TreeSet<>();

        InstructionIterator it = listing.getInstructions(text.getStart(), true);
        while (it.hasNext() && !monitor.isCancelled()) {
            Instruction ins = it.next();
            if (!text.contains(ins.getAddress())) break;
            insns++;
            String m = ins.getMnemonicString().toUpperCase();
            mnem.merge(m, 1, Integer::sum);
            Function f = fm.getFunctionContaining(ins.getAddress());
            String fname = f == null ? "<none>" : f.getEntryPoint().toString();
            String repr = ins.toString().toUpperCase();
            if (m.startsWith("F") && !m.equals("FS")) { x87++; x87Funcs.add(fname); }
            if (m.equals("FLDCW")) fldcw++;
            if (repr.contains("MM0") || repr.contains("MM1") || repr.contains("MM2") || repr.contains("MM3")
                    || repr.contains("MM4") || repr.contains("MM5") || repr.contains("MM6") || repr.contains("MM7")) {
                if (repr.contains("XMM")) sse++; else mmx++;
            }
            if (repr.contains("FS:")) fsSeg++;
            if (repr.startsWith("REP")) rep++;

            ghidra.program.model.symbol.FlowType ft = ins.getFlowType();
            if (ft.isJump() && ft.isComputed()) {
                indJmp++;
                Reference[] refs = ins.getReferencesFrom();
                long flowRefs = Arrays.stream(refs).filter(r -> r.getReferenceType().isFlow()).count();
                if (flowRefs > 0) indJmpResolved++;
                else unresolvedJmp.add(ins.getAddress() + " " + ins + " in " + fname);
            }
            if (ft.isCall() && ft.isComputed()) {
                indCall++;
                boolean viaImport = false, resolved = false;
                for (Reference r : ins.getReferencesFrom()) {
                    if (r.getReferenceType().isFlow()) resolved = true;
                    Symbol s = currentProgram.getSymbolTable().getPrimarySymbol(r.getToAddress());
                    if (s != null && (s.isExternal() || s.getParentNamespace().isExternal()
                            || r.getToAddress().getAddressSpace().isExternalSpace())) viaImport = true;
                    Data d = listing.getDataAt(r.getToAddress());
                    if (d != null && d.isPointer() && d.getValue() instanceof Address
                            && ((Address) d.getValue()).isExternalAddress()) viaImport = true;
                }
                if (viaImport) indCallViaImport++;
                else {
                    if (resolved) indCallResolved++;
                    indCallsByFunc.merge(fname, 1, Integer::sum);
                }
            }
        }

        // Code coverage: .text bytes not inside any function body.
        AddressSet funcBodies = new AddressSet();
        for (Function f : fm.getFunctions(true)) funcBodies.add(f.getBody());
        AddressSet textSet = new AddressSet(text.getStart(), text.getEnd());
        AddressSetView outside = textSet.subtract(funcBodies);
        long outsideBytes = outside.getNumAddresses();
        long outsideInsnBytes = 0, outsideDataBytes = 0;
        for (AddressRange r : outside) {
            for (Address a = r.getMinAddress(); a.compareTo(r.getMaxAddress()) <= 0; a = a.next()) {
                CodeUnit cu = listing.getCodeUnitAt(a);
                if (cu instanceof Instruction) outsideInsnBytes += cu.getLength();
                else if (cu instanceof Data && ((Data) cu).isDefined()) outsideDataBytes += cu.getLength();
                if (a.equals(r.getMaxAddress())) break;
            }
        }

        try (PrintWriter w = new PrintWriter(new FileWriter(outDir + "/recomp_survey.txt"))) {
            w.println("program\t" + currentProgram.getName());
            w.println("functions\t" + fm.getFunctionCount());
            w.println("instructions_in_text\t" + insns);
            w.println("x87_instructions\t" + x87);
            w.println("x87_functions\t" + x87Funcs.size());
            w.println("fldcw\t" + fldcw);
            w.println("mmx_instructions\t" + mmx);
            w.println("sse_instructions\t" + sse);
            w.println("fs_segment_accesses\t" + fsSeg);
            w.println("rep_string_instructions\t" + rep);
            w.println("indirect_jumps\t" + indJmp);
            w.println("indirect_jumps_resolved\t" + indJmpResolved);
            w.println("indirect_calls_total\t" + indCall);
            w.println("indirect_calls_via_import_thunks\t" + indCallViaImport);
            w.println("indirect_calls_other\t" + (indCall - indCallViaImport));
            w.println("indirect_calls_other_with_known_target\t" + indCallResolved);
            w.println("text_bytes\t" + text.getSize());
            w.println("text_bytes_outside_functions\t" + outsideBytes);
            w.println("  of_which_instructions\t" + outsideInsnBytes);
            w.println("  of_which_defined_data\t" + outsideDataBytes);
            w.println();
            w.println("# unresolved indirect jumps");
            unresolvedJmp.forEach(w::println);
            w.println();
            w.println("# non-import indirect calls per function");
            indCallsByFunc.forEach((k, v) -> w.println(k + "\t" + v));
            w.println();
            w.println("# mnemonic counts");
            mnem.forEach((k, v) -> w.println(k + "\t" + v));
        }
        println("survey written");
    }
}
