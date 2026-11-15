// Native Java compiler API adapter. No third-party parser or build tool.
// Analyze only: annotation processors disabled, empty class/source paths,
// no class generation, and no execution of any user-supplied Java class.
import java.io.File;
import java.net.URLEncoder;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import javax.tools.*;
import javax.lang.model.element.Element;
import com.sun.source.tree.*;
import com.sun.source.util.*;

public class ExcessionJavaNative {
    private static String encode(String text) {
        return URLEncoder.encode(text, StandardCharsets.UTF_8);
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 3) {
            throw new IllegalArgumentException("expected checkout, empty classpath, and sources");
        }
        Path checkout = Paths.get(args[0]).toRealPath();
        Path emptyClasspath = Paths.get(args[1]).toRealPath();
        ArrayList<Path> files = new ArrayList<>();
        TreeSet<String> selected = new TreeSet<>();
        for (int i = 2; i < args.length; i++) {
            Path file = Paths.get(args[i]).toRealPath();
            if (!file.startsWith(checkout)) {
                throw new IllegalArgumentException("Java source outside selected checkout");
            }
            files.add(file);
            selected.add(checkout.relativize(file).toString().replace(File.separatorChar, '/'));
        }
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) {
            throw new IllegalStateException("JDK compiler module unavailable");
        }
        DiagnosticCollector<JavaFileObject> diagnostics = new DiagnosticCollector<>();
        try (StandardJavaFileManager manager =
                 compiler.getStandardFileManager(diagnostics, null, StandardCharsets.UTF_8)) {
            var sources = manager.getJavaFileObjectsFromPaths(files);
            List<String> options = List.of(
                "-proc:none", "-implicit:none",
                "-classpath", emptyClasspath.toString(),
                "-sourcepath", emptyClasspath.toString(), "-encoding", "UTF-8"
            );
            JavacTask task = (JavacTask) compiler.getTask(
                null, manager, diagnostics, options, null, sources
            );
            ArrayList<CompilationUnitTree> units = new ArrayList<>();
            for (var unit : task.parse()) {
                units.add(unit);
            }
            task.analyze();
            int failures = 0;
            for (var diagnostic : diagnostics.getDiagnostics()) {
                if (diagnostic.getKind() == Diagnostic.Kind.ERROR && failures++ < 5) {
                    System.err.println("javac: " + diagnostic.getMessage(Locale.ROOT));
                }
            }
            if (failures > 0) {
                System.exit(3);
            }
            Trees trees = Trees.instance(task);
            TreeSet<String> edges = new TreeSet<>();
            for (var unit : units) {
                String from = checkout.relativize(
                    Paths.get(unit.getSourceFile().toUri()).toRealPath()
                ).toString().replace(File.separatorChar, '/');
                new TreePathScanner<Void, Void>() {
                    private void observe() {
                        Element declaration = trees.getElement(getCurrentPath());
                        while (declaration != null) {
                            TreePath declarationPath = trees.getPath(declaration);
                            if (declarationPath != null) {
                                try {
                                    Path file = Paths.get(
                                        declarationPath.getCompilationUnit().getSourceFile().toUri()
                                    ).toRealPath();
                                    if (file.startsWith(checkout)) {
                                        String to = checkout.relativize(file).toString()
                                            .replace(File.separatorChar, '/');
                                        if (!from.equals(to) && selected.contains(to)) {
                                            edges.add(from + "\t" + to);
                                        }
                                    }
                                } catch (Exception e) {
                                    throw new IllegalStateException(
                                        "invalid compiler source location", e
                                    );
                                }
                                break;
                            }
                            declaration = declaration.getEnclosingElement();
                        }
                    }
                    @Override public Void visitIdentifier(IdentifierTree node, Void unused) {
                        observe();
                        return super.visitIdentifier(node, unused);
                    }
                    @Override public Void visitMemberSelect(MemberSelectTree node, Void unused) {
                        observe();
                        return super.visitMemberSelect(node, unused);
                    }
                    @Override public Void visitMemberReference(MemberReferenceTree node, Void unused) {
                        observe();
                        return super.visitMemberReference(node, unused);
                    }
                    @Override public Void visitNewClass(NewClassTree node, Void unused) {
                        observe();
                        return super.visitNewClass(node, unused);
                    }
                }.scan(unit, null);
            }
            System.out.println("JAVA-NATIVE-V1\t" + System.getProperty("java.version"));
            for (String edge : edges) {
                String[] endpoints = edge.split("\t", -1);
                System.out.println(encode(endpoints[0]) + "\t" + encode(endpoints[1]));
            }
        }
    }
}
