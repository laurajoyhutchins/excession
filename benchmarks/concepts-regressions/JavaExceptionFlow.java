// Copyright 2026. MIT. JDK compiler AST source observer, no source execution.
// This adapter is deliberately conservative and makes no type-resolution claim.
import com.sun.source.tree.*;
import com.sun.source.util.*;
import javax.tools.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

public final class JavaExceptionFlow {
    static final int NORMAL=1, THROW=2, RETURN=4, DEPTH=4;
    final Map<String,MethodTree> methods=new TreeMap<String,MethodTree>();
    Trees trees;

    static String signature(String owner,MethodTree m) {
        StringBuilder key=new StringBuilder(owner).append('#').append(m.getName()).append('(');
        for(VariableTree p:m.getParameters()){
            if(key.charAt(key.length()-1)!='(')key.append(',');
            key.append(p.getType().toString().replaceAll("\\s+",""));
        }
        return key.append(')').toString();
    }
    void index(CompilationUnitTree unit){
        new TreePathScanner<Void,Void>(){
            final Deque<String> owners=new ArrayDeque<String>();
            @Override public Void visitClass(ClassTree t,Void x){
                owners.addLast(t.getSimpleName().toString());
                super.visitClass(t,x);owners.removeLast();return null;
            }
            @Override public Void visitMethod(MethodTree m,Void x){
                methods.put(signature(String.join(".",owners),m),m);
                return super.visitMethod(m,x);
            }
        }.scan(unit,null);
    }
    static String ident(ExpressionTree e){
        return e instanceof IdentifierTree?((IdentifierTree)e).getName().toString():null;
    }
    static String callName(ExpressionTree e){
        if(e instanceof IdentifierTree)return ident(e);
        if(e instanceof MemberSelectTree){
            MemberSelectTree m=(MemberSelectTree)e;
            if("this".equals(ident(m.getExpression())))return m.getIdentifier().toString();
        }
        return null;
    }
    MethodTree helper(String owner,String name,int arity){
        if(name==null)return null;
        MethodTree found=null;
        for(Map.Entry<String,MethodTree> e:methods.entrySet()){
            if(e.getKey().startsWith(owner+"#"+name+"(")
               &&e.getValue().getParameters().size()==arity){
                if(found!=null)return null; // Ambiguous overload, no inference.
                found=e.getValue();
            }
        }
        return found;
    }
    boolean returnsOnlyConstructedObjects(MethodTree m){
        if(m==null||m.getBody()==null)return false;
        final boolean[] seen={false},valid={true};
        new TreeScanner<Void,Void>(){
            @Override public Void visitReturn(ReturnTree r,Void x){
                seen[0]=true;
                if(!(r.getExpression() instanceof NewClassTree))valid[0]=false;
                return null;
            }
            @Override public Void visitLambdaExpression(LambdaExpressionTree t,Void x){return null;}
            @Override public Void visitClass(ClassTree t,Void x){return null;}
        }.scan(m.getBody(),null);
        return seen[0]&&valid[0];
    }
    static class Context{
        final Set<String> nonnull=new HashSet<String>();
        Context copy(){Context c=new Context();c.nonnull.addAll(nonnull);return c;}
    }
    static boolean isNull(ExpressionTree e){
        return e instanceof LiteralTree&&((LiteralTree)e).getValue()==null;
    }
    static boolean nonnullCondition(ExpressionTree e,String name){
        while(e instanceof ParenthesizedTree)e=((ParenthesizedTree)e).getExpression();
        if(!(e instanceof BinaryTree))return false;
        BinaryTree b=(BinaryTree)e;
        return b.getKind()==Tree.Kind.NOT_EQUAL_TO&&
            ((name.equals(ident(b.getLeftOperand()))&&isNull(b.getRightOperand()))||
             (name.equals(ident(b.getRightOperand()))&&isNull(b.getLeftOperand())));
    }
    int expression(ExpressionTree e,String owner,Context context,Set<String> stack,int depth){
        MethodInvocationTree call=e instanceof MethodInvocationTree?(MethodInvocationTree)e:null;
        if(e instanceof AssignmentTree){
            AssignmentTree a=(AssignmentTree)e;
            String variable=ident(a.getVariable());
            if(a.getExpression() instanceof MethodInvocationTree){
                call=(MethodInvocationTree)a.getExpression();
                MethodTree def=helper(owner,callName(call.getMethodSelect()),call.getArguments().size());
                if(variable!=null){
                    if(returnsOnlyConstructedObjects(def))context.nonnull.add(variable);
                    else context.nonnull.remove(variable);
                }
            }else if(variable!=null){
                if(a.getExpression() instanceof NewClassTree)context.nonnull.add(variable);
                else context.nonnull.remove(variable);
            }
        }
        if(call==null||depth>=DEPTH)return NORMAL;
        MethodTree target=helper(owner,callName(call.getMethodSelect()),call.getArguments().size());
        if(target==null||target.getBody()==null)return NORMAL;
        String key=signature(owner,target);
        if(!stack.add(key))return NORMAL;
        Context args=new Context();
        for(int i=0;i<call.getArguments().size();i++){
            String variable=ident(call.getArguments().get(i));
            if(variable!=null&&context.nonnull.contains(variable))
                args.nonnull.add(target.getParameters().get(i).getName().toString());
        }
        int exits=flow(target.getBody(),owner,args,stack,depth+1);
        stack.remove(key);
        return (exits&THROW)|((exits&(NORMAL|RETURN))!=0?NORMAL:0);
    }
    int flow(StatementTree s,String owner,Context context,Set<String> stack,int depth){
        if(s==null)return NORMAL;
        switch(s.getKind()){
        case THROW: return THROW;
        case RETURN: return RETURN;
        // A jump can bypass a subsequent throw. Treat it as non-throwing
        // abrupt completion instead of incorrectly proving THROW.
        case BREAK: case CONTINUE: return RETURN;
        case BLOCK:{
            int exits=NORMAL;
            for(StatementTree part:((BlockTree)s).getStatements()){
                if((exits&NORMAL)==0)break;
                exits=(exits&~NORMAL)|flow(part,owner,context,stack,depth);
            }
            return exits;
        }
        case EXPRESSION_STATEMENT:
            return expression(((ExpressionStatementTree)s).getExpression(),owner,context,stack,depth);
        case VARIABLE:{
            VariableTree v=(VariableTree)s;
            if(v.getInitializer() instanceof NewClassTree)context.nonnull.add(v.getName().toString());
            return NORMAL;
        }
        case IF:{
            IfTree b=(IfTree)s;
            for(String name:context.nonnull)
                if(nonnullCondition(b.getCondition(),name))
                    return flow(b.getThenStatement(),owner,context.copy(),stack,depth);
            return flow(b.getThenStatement(),owner,context.copy(),stack,depth)|
                   flow(b.getElseStatement(),owner,context.copy(),stack,depth);
        }
        case TRY:{
            TryTree t=(TryTree)s;
            int base=flow(t.getBlock(),owner,context,stack,depth);
            if(!t.getCatches().isEmpty()&&(base&THROW)!=0)
                for(CatchTree c:t.getCatches())
                    base|=flow(c.getBlock(),owner,context.copy(),stack,depth);
            if(t.getFinallyBlock()!=null){
                int fin=flow(t.getFinallyBlock(),owner,context,stack,depth);
                return (fin&~NORMAL)|((fin&NORMAL)!=0?base:0);
            }
            return base;
        }
        case SYNCHRONIZED:
            return flow(((SynchronizedTree)s).getBlock(),owner,context,stack,depth);
        case LABELED_STATEMENT:
            return flow(((LabeledStatementTree)s).getStatement(),owner,context,stack,depth);
        default: return NORMAL; // Unsupported Java form could complete normally.
        }
    }
    static int directThrowCount(BlockTree b){
        final int[] count={0};
        new TreeScanner<Void,Void>(){
            @Override public Void visitThrow(ThrowTree t,Void x){count[0]++;return null;}
            @Override public Void visitCatch(CatchTree t,Void x){return null;}
            @Override public Void visitLambdaExpression(LambdaExpressionTree t,Void x){return null;}
            @Override public Void visitClass(ClassTree t,Void x){return null;}
        }.scan(b,null);
        return count[0];
    }
    static String quote(String text){
        StringBuilder s=new StringBuilder("\"");
        for(char c:text.toCharArray()){
            if(c=='\\')s.append("\\\\");
            else if(c=='"')s.append("\\\"");
            else if(c=='\n')s.append("\\n");
            else if(c=='\r')s.append("\\r");
            else if(c=='\t')s.append("\\t");
            else if(c<32)s.append(String.format("\\u%04x",(int)c));
            else s.append(c);
        }
        return s.append('"').toString();
    }
    void emit(CompilationUnitTree unit,String source){
        System.out.println("{\"schema\":\"excession.java.exception-flow.observation.v1\",\"kind\":\"SOURCE\",\"source\":"+quote(source)+"}");
        new TreePathScanner<Void,Void>(){
            final Deque<String> owners=new ArrayDeque<String>();
            String method;
            int ordinal;
            @Override public Void visitClass(ClassTree t,Void x){
                owners.addLast(t.getSimpleName().toString());
                super.visitClass(t,x);owners.removeLast();return null;
            }
            @Override public Void visitMethod(MethodTree m,Void x){
                String previous=method;int oldOrdinal=ordinal;
                method=signature(String.join(".",owners),m);ordinal=0;
                super.visitMethod(m,x);
                method=previous;ordinal=oldOrdinal;return null;
            }
            @Override public Void visitCatch(CatchTree c,Void x){
                if(method!=null){
                    Context context=new Context();
                    context.nonnull.add(c.getParameter().getName().toString());
                    int exits=flow(c.getBlock(),String.join(".",owners),context,
                                   new HashSet<String>(),0);
                    long start=trees.getSourcePositions().getStartPosition(unit,c);
                    long line=start<0?0:unit.getLineMap().getLineNumber(start);
                    System.out.println("{\"schema\":\"excession.java.exception-flow.observation.v1\","+
                        "\"kind\":\"CATCH\",\"source\":"+quote(source)+
                        ",\"method\":"+quote(method)+",\"catch_index\":"+(ordinal++)+
                        ",\"caught\":"+quote(c.getParameter().getType().toString())+
                        ",\"line\":"+line+",\"direct_throw_count\":"+directThrowCount(c.getBlock())+
                        ",\"modeled_exits\":"+exits+
                        ",\"guaranteed_throw\":"+(exits==THROW)+"}");
                }
                return super.visitCatch(c,x);
            }
        }.scan(unit,null);
    }
    public static void main(String[] paths)throws Exception{
        if(paths.length<1||paths.length>512)
            throw new IllegalArgumentException("expected 1..512 relative Java source paths");
        JavaCompiler compiler=ToolProvider.getSystemJavaCompiler();
        if(compiler==null)throw new IllegalStateException("JDK compiler unavailable");
        DiagnosticCollector<JavaFileObject> errors=new DiagnosticCollector<JavaFileObject>();
        long total=0;
        try(StandardJavaFileManager manager=compiler.getStandardFileManager(errors,Locale.ROOT,null)){
            for(String path:paths){
                Path source=Paths.get(path);
                if(source.isAbsolute()||path.contains("..")||!path.endsWith(".java")||
                   Files.isSymbolicLink(source)||!Files.isRegularFile(source,LinkOption.NOFOLLOW_LINKS)||
                   Files.size(source)>1_048_576L)
                    throw new IllegalArgumentException("unsupported Java source file: "+path);
                total+=Files.size(source);
                if(total>32L*1024*1024)throw new IllegalArgumentException("source byte bound exceeded");
                JavacTask task=(JavacTask)compiler.getTask(null,manager,errors,
                    Arrays.asList("-proc:none"),null,
                    manager.getJavaFileObjects(new File(path)));
                JavaExceptionFlow adapter=new JavaExceptionFlow();
                adapter.trees=Trees.instance(task);
                for(CompilationUnitTree unit:task.parse()){
                    adapter.index(unit);
                    adapter.emit(unit,path);
                }
            }
        }
        for(Diagnostic<? extends JavaFileObject> error:errors.getDiagnostics())
            if(error.getKind()==Diagnostic.Kind.ERROR)
                throw new IllegalStateException("Java parse error at line "+
                     error.getLineNumber()+": "+error.getMessage(Locale.ROOT));
    }
}
