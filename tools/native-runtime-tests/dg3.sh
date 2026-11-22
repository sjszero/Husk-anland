cd /tmp
cat > t.c <<'C'
#include <stdio.h>
#include <unistd.h>
#include <sys/wait.h>
#include <errno.h>
int main(void){
  pid_t p=fork();
  if(p<0){ printf("fork err %d\n", errno); return 1; }
  if(p==0){ execl("/usr/bin/true","true",(char*)0); printf("execl err %d\n", errno); _exit(127); }
  int st=0; pid_t w=waitpid(p,&st,0);
  printf("wait=%d st=%d exited=%d sig=%d code=%d\n",(int)w,st,WIFEXITED(st),WIFSIGNALED(st)?WTERMSIG(st):0,WIFEXITED(st)?WEXITSTATUS(st):-1);
  return 0;
}
C
gcc -O2 -o t t.c && ./t; echo "rc=$?"
echo "--- posix_spawn route ---"
cat > s.c <<'C'
#include <spawn.h>
#include <stdio.h>
#include <sys/wait.h>
extern char **environ;
int main(void){ char *av[]={"true",0}; pid_t p; int r=posix_spawn(&p,"/usr/bin/true",0,0,av,environ);
  int st=0; if(r==0) waitpid(p,&st,0);
  printf("spawn r=%d st=%d code=%d\n", r, st, WIFEXITED(st)?WEXITSTATUS(st):-1); return 0; }
C
gcc -O2 -o s s.c && ./s; echo "rc=$?"
